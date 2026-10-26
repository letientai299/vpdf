mod overlay;
mod palette;
use crate::style::{ColorChange, Role, StyleChange, Tone};
pub use overlay::{Overlay, Paint};
pub use palette::Palette;
use serde::{Deserialize, Serialize};

#[derive(
  Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
  #[default]
  Default,
  TokyonightNight,
  Dracula,
  GithubLight,
  GithubDark,
  Nord,
}

impl Theme {
  pub const ALL: &[Self] = &[
    Self::Default,
    Self::TokyonightNight,
    Self::Dracula,
    Self::GithubLight,
    Self::GithubDark,
    Self::Nord,
  ];

  pub const fn name(self) -> &'static str {
    match self {
      Self::Default => "default",
      Self::TokyonightNight => "tokyonight-night",
      Self::Dracula => "dracula",
      Self::GithubLight => "github-light",
      Self::GithubDark => "github-dark",
      Self::Nord => "nord",
    }
  }

  pub fn parse(name: &str) -> Option<Self> {
    Self::ALL.iter().copied().find(|theme| theme.name() == name)
  }

  pub fn style(self, role: Role) -> StyleChange {
    let change = role.style();
    if self == Self::Default {
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
        if role == Role::TextLanding {
          patch.foreground = ColorChange::Set(Tone::Black);
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
      Tone::Blue | Tone::PreviewBorder => palette.blue,
      Tone::Magenta => palette.magenta,
      Tone::Yellow => palette.yellow,
      Tone::Cyan | Tone::ActiveBar => palette.cyan,
      Tone::Bar if background => palette.surface,
      Tone::Bar => palette.muted,
      Tone::Error => palette.red,
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
