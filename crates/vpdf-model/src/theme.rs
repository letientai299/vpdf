mod overlay;
mod palette;
use crate::style::{Attribute, ColorChange, Role, StyleChange, Tone};
pub use overlay::{Overlay, Paint};
pub use palette::{Palette, is_light, luminance};
use serde::{Deserialize, Serialize};

#[derive(
  Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
  #[default]
  System,
  Light,
  Dark,
}

impl Theme {
  pub const EXPECTED: &str =
    "expected system, light, dark, or a path; see etc/themes/";

  pub const ALL: &[Self] = &[Self::System, Self::Light, Self::Dark];

  pub const fn name(self) -> &'static str {
    match self {
      Self::System => "system",
      Self::Light => "light",
      Self::Dark => "dark",
    }
  }

  pub fn parse(name: &str) -> Option<Self> {
    Self::ALL.iter().copied().find(|theme| theme.name() == name)
  }

  pub fn palette(self) -> Palette {
    match self {
      Self::Light => Palette::light(),
      Self::System | Self::Dark => Palette::dark(),
    }
  }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Colors {
  #[default]
  Terminal,
  Palette(Palette),
}

impl Colors {
  pub fn palette(self) -> Palette {
    match self {
      Self::Terminal => Palette::dark(),
      Self::Palette(palette) => palette,
    }
  }

  pub fn style(self, role: Role) -> StyleChange {
    let change = role.style();
    if self == Self::Terminal {
      return change;
    }
    match change {
      StyleChange::Reset(mut patch) => {
        if patch.foreground == ColorChange::Keep {
          patch.foreground = ColorChange::Default;
        }
        if patch.background == ColorChange::Keep {
          patch.background = ColorChange::Default;
        }
        match role {
          Role::TextLanding => patch.foreground = ColorChange::Set(Tone::Black),
          Role::TextCursorline => {
            patch.background =
              ColorChange::Set(Tone::Rgb(self.palette().cursorline));
          }
          Role::Dim => {
            patch.foreground = ColorChange::Set(Tone::Bar);
            patch.attributes = &[];
          }
          Role::HoverDim => {
            patch.foreground = ColorChange::Set(Tone::Bar);
            patch.attributes = &[Attribute::Underline];
          }
          _ => {}
        }
        StyleChange::Reset(patch)
      }
      StyleChange::Patch(patch) => StyleChange::Patch(patch),
    }
  }

  pub fn color(self, tone: Tone, background: bool) -> [u8; 3] {
    let palette = self.palette();
    match tone {
      Tone::Black | Tone::BrightWhite => palette.background,
      Tone::White | Tone::PreviewText => palette.foreground,
      Tone::Blue | Tone::PreviewBorder => palette.accent,
      Tone::Magenta => palette.highlight,
      Tone::Yellow => palette.r#match,
      Tone::Cyan | Tone::ActiveBar => palette.link,
      Tone::Bar if background => palette.surface,
      Tone::Bar => palette.muted,
      Tone::Error => palette.error,
      Tone::Rgb(rgb) => rgb,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn names_round_trip() {
    for &theme in Theme::ALL {
      assert_eq!(Theme::parse(theme.name()), Some(theme));
      let json = serde_json::to_string(&theme).unwrap();
      assert_eq!(json, format!("\"{}\"", theme.name()));
      assert_eq!(serde_json::from_str::<Theme>(&json).unwrap(), theme);
    }
    assert_eq!(Theme::parse("github"), None);
    assert!(serde_json::from_str::<Theme>("\"unknown\"").is_err());
  }
}
