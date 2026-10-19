//! What the interactive reader requires of the terminal path: Kitty
//! graphics, Kitty keyboard disambiguation, and cells measured in
//! pixels. `:text` presents extracted text on such a path; it is no
//! fallback, so the same gate applies.
use crate::Result;
use crate::layout::Screen;
use std::fmt;
use vpdf_terminal::caps::{Caps, KeyLack, Lack, Profile};
use vpdf_terminal::terminal::{self, Terminal};

/// A requirement the terminal path misses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Missing {
  Graphics(Lack),
  Keyboard(KeyLack),
  /// No cell size in pixels, which places every image.
  Pixels,
}

impl fmt::Display for Missing {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Self::Graphics(lack) => lack.fmt(f),
      Self::Keyboard(lack) => lack.fmt(f),
      Self::Pixels => f.write_str(terminal::NO_CELL),
    }
  }
}

/// Every missing requirement, one line each.
#[derive(Debug, Eq, PartialEq)]
pub struct Unsupported(Vec<Missing>);

impl fmt::Display for Unsupported {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    for (index, missing) in self.0.iter().enumerate() {
      let separator = if index == 0 { "" } else { "\n" };
      write!(f, "{separator}{missing}")?;
    }
    Ok(())
  }
}

impl std::error::Error for Unsupported {}

/// What `caps` misses, given the keyboard negotiation's verdict and
/// whether cells measure in pixels.
fn missing(caps: &Caps, keys: Option<KeyLack>, pixels: bool) -> Vec<Missing> {
  let graphics = match caps.profile() {
    Profile::Graphics | Profile::CropQuirk => None,
    Profile::Unsupported(lack) => Some(Missing::Graphics(lack)),
  };
  // A terminal silent to DA1 fails both for one reason.
  let keys = keys.filter(|lack| {
    !(*lack == KeyLack::Unanswered
      && graphics == Some(Missing::Graphics(Lack::Answer)))
  });
  let pixels = (!pixels).then_some(Missing::Pixels);
  graphics
    .into_iter()
    .chain(keys.map(Missing::Keyboard))
    .chain(pixels)
    .collect()
}

/// Takes the terminal for the interactive reader when the path meets
/// every requirement; otherwise gives it back and names each one
/// missing.
pub fn enter() -> Result<(Terminal, Screen)> {
  // `resume` probes, then fails with the keyboard's reason.
  let (terminal, keys) = match Terminal::enter() {
    Ok(terminal) => (Some(terminal), None),
    Err(error) => (None, Some(error.downcast::<KeyLack>()?)),
  };
  let caps = vpdf_terminal::caps::probe()?;
  let missing = missing(caps, keys, terminal::cell().is_some());
  match terminal {
    Some(terminal) if missing.is_empty() => Ok((terminal, terminal::size()?)),
    _ => Err(Unsupported(missing).into()),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_terminal::caps::{Graphics, Host, Support, Tmux};

  fn caps(host: Host, version: &str, answered: bool) -> Caps {
    Caps {
      host,
      version: Some(version.into()),
      keyboard: Some(0),
      sync: Support::Missing,
      pixels: Support::Missing,
      resize: Support::Missing,
      pointer: Support::Missing,
      drop: Support::Missing,
      clipboard: None,
      cell: None,
      attributes: answered.then(|| vec![62]),
    }
  }

  /// (label, caps, keyboard verdict, pixels, lines)
  type Case<'a> = (&'a str, &'a Caps, Option<KeyLack>, bool, &'a [&'a str]);

  #[test]
  fn each_missing_requirement_has_its_line() {
    let kitty = caps(Host::Direct(Graphics::Ok), "kitty(0.49.1)", true);
    let iterm = caps(Host::Direct(Graphics::Ok), "iTerm2 3.7.6", true);
    let wezterm = "WezTerm 20240203-110809-5046fc22";
    let wezterm = caps(Host::Direct(Graphics::Ok), wezterm, true);
    let foot = caps(Host::Direct(Graphics::Silent), "foot(1.28.0)", true);
    let warp = caps(Host::Direct(Graphics::Ok), "Warp(v0.2026.03.25)", true);
    let silent = caps(Host::Direct(Graphics::Silent), "kitty(0.49.1)", false);
    let screen = caps(Host::Screen, "kitty(0.49.1)", false);
    let tmux = Tmux {
      version: vec![3, 8],
      client: Some("kitty(0.49.1)".into()),
      cell: None,
      passthrough: None,
      extended_keys: true,
      focus_events: true,
      origin: (0, 0),
    };
    let off = Tmux { extended_keys: false, ..tmux.clone() };
    let tmux = caps(Host::Tmux(Some(tmux)), "tmux 3.8", true);
    let off = caps(Host::Tmux(Some(off)), "tmux 3.8", true);
    let protocol = |name: &str| Some(KeyLack::Protocol(Some(name.into())));
    let cases: [Case; 10] = [
      ("kitty", &kitty, None, true, &[]),
      ("iTerm2 crop quirk", &iterm, None, true, &[]),
      ("tmux in kitty", &tmux, None, true, &[]),
      (
        "no pixels",
        &kitty,
        None,
        false,
        &["Terminal reports no cell size in pixels"],
      ),
      (
        "WezTerm keys off",
        &wezterm,
        protocol("WezTerm 20240203-110809-5046fc22"),
        true,
        &["WezTerm needs enable_kitty_keyboard = true"],
      ),
      ("foot", &foot, None, true, &["No Kitty graphics"]),
      (
        "Warp, every requirement",
        &warp,
        Some(KeyLack::Refused(Some(4))),
        false,
        &[
          "Warp lacks image crops and cell offsets",
          "Kitty keyboard disambiguation refused, flags 4",
          "Terminal reports no cell size in pixels",
        ],
      ),
      (
        "tmux keys",
        &off,
        Some(KeyLack::ExtendedKeys),
        true,
        &["tmux needs `set -s extended-keys on` in tmux.conf"],
      ),
      (
        "no DA1 says so once",
        &silent,
        Some(KeyLack::Unanswered),
        true,
        &["Terminal did not answer queries"],
      ),
      (
        "GNU screen",
        &screen,
        Some(KeyLack::Screen),
        true,
        &[
          "GNU screen cannot pass Kitty graphics",
          "GNU screen lacks Kitty keyboard disambiguation",
        ],
      ),
    ];
    for (label, caps, keys, pixels, lines) in cases {
      let missing = missing(caps, keys, pixels);
      assert_eq!(missing.is_empty(), lines.is_empty(), "{label}");
      assert_eq!(Unsupported(missing).to_string(), lines.join("\n"), "{label}");
    }
  }
}
