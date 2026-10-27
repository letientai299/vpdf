use super::{Duration, PASSTHROUGH, fmt, join};

/// Time the terminal has to answer DA1. Answers that come later decode
/// as replies, which the reader drops, never as keys.
pub const WAIT: Duration = Duration::from_secs(1);

/// Image id of the graphics query, which stores no image. Ghostty,
/// Warp, and Rio answer only a query with an id.
pub(super) const QUERY: u32 = 31;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Caps {
  pub host: Host,
  /// XTVERSION of what answered: the terminal, or tmux itself.
  pub version: Option<String>,
  /// Kitty keyboard flags the probe found, before the reader pushes
  /// its own; None without the protocol.
  pub keyboard: Option<u32>,
  pub background: Option<[u8; 3]>,
  /// Synchronized output, mode 2026.
  pub sync: Support,
  /// SGR pixel mouse reports, mode 1016.
  pub pixels: Support,
  /// In-band resize reports, mode 2048.
  pub resize: Support,
  /// Pointer shapes, OSC 22.
  pub pointer: Support,
  /// Drag and drop, OSC 72.
  pub drop: Support,
  /// Mode 5522; None without an answer.
  pub clipboard: Option<Support>,
  /// Cell size from `CSI 16 t`, asked only when TIOCGWINSZ has none.
  pub cell: Option<Cell>,
  /// DA1's answer; None when it missed `WAIT` or was never asked.
  pub attributes: Option<Vec<u32>>,
}

/// What sits between vpdf and the terminal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Host {
  /// The terminal itself, with its answer to the graphics query.
  Direct(Graphics),
  /// tmux; None when `tmux display` failed.
  Tmux(Option<Tmux>),
  /// GNU screen, asked nothing.
  Screen,
}

/// The answer to `a=q`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Graphics {
  Ok,
  /// An error such as `EINVAL:…`.
  Error(String),
  Silent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tmux {
  /// `#{version}` as numbers: `3.2a` is 3.2, `next-3.9` is 3.9; empty
  /// for a name without a tmux release, such as `openbsd-7.8`.
  pub version: Vec<u32>,
  /// The client terminal's XTVERSION, `#{client_termtype}`.
  pub client: Option<String>,
  /// The client's cell in pixels at the probe; cell sizes read it
  /// fresh through `tmux_now`.
  pub cell: Option<Cell>,
  /// `allow-passthrough` for vpdf's pane; None before tmux 3.3.
  pub passthrough: Option<Passthrough>,
  /// `extended-keys` is `on` or `always`, so the pane may ask for
  /// modifyOtherKeys and tell Ctrl chords from their legacy bytes.
  pub extended_keys: bool,
  /// `focus-events`: tmux tells panes when they gain or lose focus.
  pub focus_events: bool,
  /// The pane's top-left cell on the client at the probe: past a status
  /// line on top, less the offset into a window larger than the
  /// client. Screens read it fresh through `tmux_now`.
  pub origin: (u16, u16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Passthrough {
  Off,
  On,
  /// Also from panes that are not visible.
  All,
}

/// Cell size in pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cell {
  pub width: u16,
  pub height: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Support {
  /// The terminal answered that it has it.
  Answered,
  /// The terminal's name says so; it does not answer the query.
  Named,
  Missing,
}

/// How the reader can draw on this terminal. Keyboard support is a
/// separate requirement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Profile {
  Graphics,
  /// iTerm2 stretches a crop to the whole image unless `c,r` size it.
  CropQuirk,
  Unsupported(Lack),
}

/// Why graphics are unsupported.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Lack {
  Screen,
  /// DA1 missed `WAIT`.
  Answer,
  /// No OK to the graphics query; the error when one came.
  Graphics(Option<String>),
  /// A named terminal lacks what the reader draws with.
  Terminal(&'static str),
  /// `tmux display` failed.
  Tmux,
  /// tmux's client terminal, when known, is not a graphics terminal.
  Client(Option<String>),
}

impl fmt::Display for Lack {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Self::Screen => write!(f, "GNU screen cannot pass Kitty graphics"),
      Self::Answer => write!(f, "Terminal did not answer queries"),
      Self::Graphics(None) => write!(f, "No Kitty graphics"),
      Self::Graphics(Some(error)) => {
        write!(f, "Kitty graphics failed: {error}")
      }
      Self::Terminal(reason) => write!(f, "{reason}"),
      Self::Tmux => write!(f, "Cannot query tmux"),
      Self::Client(None) => write!(f, "tmux names no client terminal"),
      Self::Client(Some(name)) => {
        write!(f, "tmux client {name} lacks graphics")
      }
    }
  }
}

impl fmt::Display for Profile {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Self::Graphics => write!(f, "Kitty graphics"),
      Self::CropQuirk => write!(f, "Kitty graphics, iTerm2 crop sizing"),
      Self::Unsupported(lack) => write!(f, "unsupported ({lack})"),
    }
  }
}

impl fmt::Display for Support {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Self::Answered => write!(f, "yes"),
      Self::Named => write!(f, "yes, by name"),
      Self::Missing => write!(f, "no"),
    }
  }
}

/// The names `allow-passthrough` takes.
impl fmt::Display for Passthrough {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    f.write_str(match self {
      Self::Off => "off",
      Self::On => "on",
      Self::All => "all",
    })
  }
}

impl fmt::Display for Cell {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(f, "{}x{} px", self.width, self.height)
  }
}

/// One `Name: value` line per field, for `:info`. Values drop control
/// characters, since a terminal's answer can hold a line break.
impl fmt::Display for Caps {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    let version = self.version.as_deref().unwrap_or("none");
    let mut rows = vec![
      ("Profile", self.profile().to_string()),
      ("XTVERSION", version.into()),
    ];
    match &self.host {
      Host::Direct(graphics) => rows.push((
        "Graphics query",
        match graphics {
          Graphics::Ok => "OK".into(),
          Graphics::Error(error) => format!("failed: {error}"),
          Graphics::Silent => "no answer".into(),
        },
      )),
      Host::Tmux(None) => rows.push(("tmux", "cannot query".into())),
      Host::Tmux(Some(tmux)) => rows.extend(tmux.rows()),
      Host::Screen => rows.push(("GNU screen", "not queried".into())),
    }
    let keyboard =
      self.keyboard.map(|flags| format!("yes, flags {flags} at start"));
    rows.extend([
      ("Kitty keyboard", keyboard.unwrap_or_else(|| "no".into())),
      ("Synchronized output", self.sync.to_string()),
      ("Pixel mouse", self.pixels.to_string()),
      ("Resize reports", self.resize.to_string()),
      ("Pointer shapes", self.pointer.to_string()),
      ("Drag and drop", self.drop.to_string()),
      (
        "Image clipboard",
        self
          .clipboard
          .map_or_else(|| "no answer".into(), |support| support.to_string()),
      ),
    ]);
    // Asked only without TIOCGWINSZ pixels; absent is the usual case.
    if let Some(cell) = self.cell {
      rows.push(("Cell reply", cell.to_string()));
    }
    let attributes = self.attributes.as_deref().map(|values| join(values, ";"));
    rows.push(("DA1", attributes.unwrap_or_else(|| "no answer".into())));
    for (index, (name, value)) in rows.into_iter().enumerate() {
      let value: String = crate::text::printable(&value).collect();
      let separator = if index == 0 { "" } else { "\n" };
      write!(f, "{separator}{name}: {value}")?;
    }
    Ok(())
  }
}

impl Tmux {
  pub(super) fn rows(&self) -> [(&'static str, String); 6] {
    let release = join(&self.version, ".");
    let on = |yes| if yes { "on" } else { "off" };
    [
      (
        "tmux",
        if release.is_empty() { "unknown release".into() } else { release },
      ),
      ("tmux client", self.client.clone().unwrap_or_else(|| "none".into())),
      (
        "tmux cell",
        self.cell.map_or_else(|| "none".into(), |cell| cell.to_string()),
      ),
      ("tmux passthrough", self.passthrough_row(PASSTHROUGH.get().copied())),
      ("tmux extended keys", on(self.extended_keys).into()),
      ("tmux focus events", on(self.focus_events).into()),
    ]
  }

  /// `allow-passthrough` at start, and what vpdf `set` for its pane.
  pub(super) fn passthrough_row(&self, set: Option<Passthrough>) -> String {
    let start = match self.passthrough {
      Some(passthrough) => passthrough.to_string(),
      None if self.before(&[3, 3]) => "on, no option before tmux 3.3".into(),
      None => "unknown".into(),
    };
    match set {
      Some(set) => format!("{start} at start, Vpdf set {set}"),
      None => start,
    }
  }

  /// Whether tmux's version comes before `release`; unknown releases,
  /// such as a distribution's own, count as new.
  pub fn before(&self, release: &[u32]) -> bool {
    !self.version.is_empty() && self.version[..] < release[..]
  }

  /// Hidden-pane cleanup needs `all` passthrough.
  pub(super) fn wanted(&self) -> Option<Passthrough> {
    match self.passthrough? {
      Passthrough::All => None,
      Passthrough::Off | Passthrough::On => Some(Passthrough::All),
    }
  }
}
