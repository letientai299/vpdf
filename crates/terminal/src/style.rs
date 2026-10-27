use std::io::{self, Write};
use vpdf_model::style::{
  Attribute, Color, ColorChange, Role, StyleChange, Tone,
};
use vpdf_model::theme::Colors;

pub const fn sgr(role: Role) -> &'static str {
  macro_rules! encoded {
    ($role:expr) => {{
      const ENCODED: Encoded = encode($role.style());
      const TEXT: &str = ENCODED.text();
      TEXT
    }};
  }
  match role {
    Role::Reset => encoded!(Role::Reset),
    Role::Bar => encoded!(Role::Bar),
    Role::Selected => encoded!(Role::Selected),
    Role::StatusFile => encoded!(Role::StatusFile),
    Role::TabActive => encoded!(Role::TabActive),
    Role::TabInactive => encoded!(Role::TabInactive),
    Role::Current => encoded!(Role::Current),
    Role::Hover => encoded!(Role::Hover),
    Role::HoverDim => encoded!(Role::HoverDim),
    Role::BarHover => encoded!(Role::BarHover),
    Role::Heading => encoded!(Role::Heading),
    Role::Key => encoded!(Role::Key),
    Role::Mode => encoded!(Role::Mode),
    Role::Argument => encoded!(Role::Argument),
    Role::File => encoded!(Role::File),
    Role::Muted => encoded!(Role::Muted),
    Role::Track => encoded!(Role::Track),
    Role::Dim => encoded!(Role::Dim),
    Role::StatusSection => encoded!(Role::StatusSection),
    Role::Error => encoded!(Role::Error),
    Role::TextLanding => encoded!(Role::TextLanding),
    Role::TextCursorline => encoded!(Role::TextCursorline),
    Role::TextCursor => encoded!(Role::TextCursor),
    Role::TextSelection => encoded!(Role::TextSelection),
    Role::TextMatch => encoded!(Role::TextMatch),
    Role::HintTyped => encoded!(Role::HintTyped),
    Role::Hint => encoded!(Role::Hint),
    Role::HintPickedTyped => encoded!(Role::HintPickedTyped),
    Role::HintPicked => encoded!(Role::HintPicked),
    Role::TextLink => encoded!(Role::TextLink),
    Role::PreviewBorder => encoded!(Role::PreviewBorder),
    Role::PreviewText => encoded!(Role::PreviewText),
    Role::Badge(Color::Blue) => encoded!(Role::Badge(Color::Blue)),
    Role::Badge(Color::Magenta) => encoded!(Role::Badge(Color::Magenta)),
    Role::Badge(Color::Yellow) => encoded!(Role::Badge(Color::Yellow)),
    Role::Badge(Color::Cyan) => encoded!(Role::Badge(Color::Cyan)),
    Role::Chosen => encoded!(Role::Chosen),
    // `write` formats document colors.
    Role::Swatch(_) => "",
  }
}

pub fn write(
  out: &mut impl Write,
  role: Role,
  theme: Colors,
) -> io::Result<()> {
  let theme = if theme == Colors::Terminal
    && matches!(role, Role::PreviewBorder | Role::PreviewText)
  {
    Colors::Palette(crate::caps::palette())
  } else {
    theme
  };
  if theme == Colors::Terminal && !matches!(role, Role::Swatch(_)) {
    return out.write_all(sgr(role).as_bytes());
  }
  out.write_all(b"\x1b[")?;
  let patch = match theme.style(role) {
    StyleChange::Reset(patch) => {
      out.write_all(b"0;")?;
      patch
    }
    StyleChange::Patch(patch) => patch,
  };
  for &attribute in patch.attributes {
    write!(out, "{};", attribute_code(attribute))?;
  }
  let palette = theme.palette();
  let mut separator = "";
  for (change, background) in
    [(patch.foreground, false), (patch.background, true)]
  {
    let rgb = match change {
      ColorChange::Keep => continue,
      ColorChange::Default if background => palette.background,
      ColorChange::Default => palette.foreground,
      ColorChange::Set(tone) => theme.color(tone, background),
    };
    let code = if background { 48 } else { 38 };
    write!(out, "{separator}{code};2;{};{};{}", rgb[0], rgb[1], rgb[2])?;
    separator = ";";
  }
  out.write_all(b"m")
}

const fn attribute_code(attribute: Attribute) -> &'static str {
  match attribute {
    Attribute::Bold => "1",
    Attribute::Dim => "2",
    Attribute::Italic => "3",
    Attribute::Underline => "4",
    Attribute::Inverse => "7",
  }
}

struct Encoded {
  bytes: [u8; 96],
  len: usize,
}

impl Encoded {
  const fn text(&self) -> &str {
    match str::from_utf8(self.bytes.split_at(self.len).0) {
      Ok(text) => text,
      Err(_) => panic!("invalid style encoding"),
    }
  }

  const fn push(&mut self, text: &str) {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
      self.bytes[self.len] = bytes[index];
      self.len += 1;
      index += 1;
    }
  }

  const fn parameter(&mut self, text: &str) {
    if self.len > 2 {
      self.push(";");
    }
    self.push(text);
  }

  const fn color(&mut self, change: ColorChange, background: bool) {
    let value = match change {
      ColorChange::Keep => return,
      ColorChange::Default => {
        if background {
          "49"
        } else {
          "39"
        }
      }
      ColorChange::Set(tone) => palette(tone, background),
    };
    self.parameter(value);
  }
}

const fn encode(change: StyleChange) -> Encoded {
  let mut out = Encoded { bytes: [0; 96], len: 0 };
  out.push("\x1b[");
  let patch = match change {
    StyleChange::Reset(patch) => {
      out.parameter("0");
      patch
    }
    StyleChange::Patch(patch) => patch,
  };
  let mut index = 0;
  while index < patch.attributes.len() {
    out.parameter(attribute_code(patch.attributes[index]));
    index += 1;
  }
  out.color(patch.foreground, false);
  out.color(patch.background, true);
  if out.len == 2 {
    out.len = 0;
    return out;
  }
  out.push("m");
  out
}

const fn palette(tone: Tone, background: bool) -> &'static str {
  match (tone, background) {
    (Tone::Black, false) => "30",
    (Tone::Black, true) => "40",
    (Tone::White | Tone::PreviewText, false) => "37",
    (Tone::White | Tone::PreviewText, true) => "47",
    (Tone::BrightWhite, false) => "97",
    (Tone::BrightWhite, true) => "107",
    (Tone::Blue | Tone::PreviewBorder, false) => "34",
    (Tone::Blue | Tone::PreviewBorder, true) => "44",
    (Tone::Magenta, false) => "35",
    (Tone::Magenta, true) => "45",
    (Tone::Yellow, false) => "33",
    (Tone::Yellow, true) => "43",
    (Tone::Cyan, false) => "36",
    (Tone::Cyan, true) => "46",
    (Tone::Bar | Tone::Track | Tone::SelectionBg, false) => "90",
    (Tone::Bar | Tone::Track | Tone::SelectionBg, true) => "100",
    (Tone::ActiveBar, false) => "94",
    (Tone::ActiveBar, true) => "104",
    (Tone::Error, false) => "31",
    (Tone::Error, true) => "41",
    // Only `write` sets document colors.
    (Tone::Rgb(_), false) => "39",
    (Tone::Rgb(_), true) => "49",
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use sha2::{Digest, Sha256};

  #[test]
  fn system_previews_use_palette() -> io::Result<()> {
    let p = crate::caps::palette();
    for (role, foreground) in
      [(Role::PreviewBorder, p.accent), (Role::PreviewText, p.foreground)]
    {
      let mut out = Vec::new();
      write(&mut out, role, Colors::Terminal)?;
      assert_eq!(
        String::from_utf8(out).unwrap(),
        format!(
          "\x1b[0;38;2;{};{};{};48;2;{};{};{}m",
          foreground[0],
          foreground[1],
          foreground[2],
          p.background[0],
          p.background[1],
          p.background[2]
        )
      );
    }
    Ok(())
  }

  #[test]
  fn swatches_write_their_color() -> io::Result<()> {
    for theme in
      [Colors::Terminal, Colors::Palette(vpdf_model::theme::Palette::dark())]
    {
      let mut out = Vec::new();
      write(&mut out, Role::Swatch([255, 235, 59]), theme)?;
      assert_eq!(out, b"\x1b[38;2;255;235;59m", "{theme:?}");
    }
    Ok(())
  }

  #[test]
  fn named_colors_are_explicit() -> io::Result<()> {
    for &theme in &[
      Colors::Terminal,
      Colors::Palette(vpdf_model::theme::Palette::light()),
      Colors::Palette(vpdf_model::theme::Palette::dark()),
    ] {
      let mut out = Vec::new();
      write(&mut out, Role::Reset, theme)?;
      if theme == Colors::Terminal {
        assert_eq!(out, b"\x1b[0m");
        continue;
      }
      let p = theme.palette();
      assert_eq!(
        String::from_utf8(out).unwrap(),
        format!(
          "\x1b[0;38;2;{};{};{};48;2;{};{};{}m",
          p.foreground[0],
          p.foreground[1],
          p.foreground[2],
          p.background[0],
          p.background[1],
          p.background[2]
        )
      );
      let mut out = Vec::new();
      write(&mut out, Role::Heading, theme)?;
      assert_eq!(
        String::from_utf8(out).unwrap(),
        format!("\x1b[1;38;2;{};{};{}m", p.accent[0], p.accent[1], p.accent[2])
      );
    }
    Ok(())
  }

  #[test]
  fn badges_write_mode_fills() -> io::Result<()> {
    use vpdf_model::theme::Palette;
    let dark = include_str!("../../../etc/themes/dark.toml");
    let palette =
      Palette::parse(&format!("{dark}\n[mode]\nvisual = \"#101010\"\n"))
        .map_err(io::Error::other)?;
    let mut out = Vec::new();
    write(&mut out, Role::Badge(Color::Magenta), Colors::Palette(palette))?;
    assert_eq!(out, b"\x1b[0;1;38;2;255;255;255;48;2;16;16;16m");
    Ok(())
  }

  #[test]
  fn track_and_selection_colors() -> io::Result<()> {
    use vpdf_model::theme::Palette;
    for mut palette in [Palette::light(), Palette::dark()] {
      palette.track = Some([1, 2, 3]);
      palette.selection_bg = Some([4, 5, 6]);
      let theme = Colors::Palette(palette);
      let mut out = Vec::new();
      write(&mut out, Role::Selected, theme)?;
      write(&mut out, Role::Track, theme)?;
      let [r, g, b] = palette.foreground;
      assert_eq!(
        out,
        format!("\x1b[0;1;38;2;{r};{g};{b};48;2;4;5;6m\x1b[38;2;1;2;3m")
          .as_bytes()
      );
    }
    assert_eq!(sgr(Role::Track), sgr(Role::Muted));
    assert_eq!(sgr(Role::Selected), "\x1b[0;1;100m");
    Ok(())
  }

  #[test]
  fn default_colors_are_explicit() {
    use vpdf_model::style::StylePatch;
    let keep = StylePatch {
      foreground: ColorChange::Keep,
      background: ColorChange::Keep,
      attributes: &[],
    };
    let defaults = StylePatch {
      foreground: ColorChange::Default,
      background: ColorChange::Default,
      ..keep
    };
    assert_eq!(encode(StyleChange::Patch(keep)).text(), "");
    assert_eq!(encode(StyleChange::Reset(keep)).text(), "\x1b[0m");
    assert_eq!(encode(StyleChange::Patch(defaults)).text(), "\x1b[39;49m");
    assert_eq!(encode(StyleChange::Reset(defaults)).text(), "\x1b[0;39;49m");
  }

  #[test]
  fn palette_keeps_legacy_bytes() {
    let roles = [
      Role::Reset,
      Role::Bar,
      Role::Selected,
      Role::StatusFile,
      Role::TabActive,
      Role::TabInactive,
      Role::Current,
      Role::Hover,
      Role::HoverDim,
      Role::BarHover,
      Role::Heading,
      Role::Key,
      Role::Mode,
      Role::Argument,
      Role::File,
      Role::Muted,
      Role::Dim,
      Role::StatusSection,
      Role::Error,
      Role::TextLanding,
      Role::TextCursorline,
      Role::TextCursor,
      Role::TextSelection,
      Role::TextMatch,
      Role::HintTyped,
      Role::Hint,
      Role::HintPickedTyped,
      Role::HintPicked,
      Role::TextLink,
      Role::Badge(Color::Blue),
      Role::Badge(Color::Magenta),
      Role::Badge(Color::Yellow),
      Role::Badge(Color::Cyan),
    ];
    let mut hash = Sha256::new();
    for role in roles {
      hash.update(sgr(role).as_bytes());
    }
    // ANSI roles exclude palette-backed previews.
    let expected: [u8; 32] = [
      0xe7, 0xaa, 0x0d, 0x3a, 0x20, 0xe0, 0x1b, 0x09, 0x0e, 0x68, 0x46, 0x4c,
      0x4d, 0x28, 0x68, 0x4a, 0x36, 0xf3, 0xe0, 0x64, 0x80, 0x66, 0xf1, 0xb0,
      0x20, 0xf4, 0x17, 0xd5, 0xe5, 0x89, 0xf6, 0xf6,
    ];
    assert_eq!(hash.finalize().as_slice(), expected.as_slice());
  }
}
