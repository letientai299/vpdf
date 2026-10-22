use eframe::egui::Color32;
use vpdf_model::style::{Attribute, ColorChange, Role, StyleChange, Tone};

pub const BACKGROUND: Color32 = Color32::from_rgb(24, 27, 36);
pub const FOREGROUND: Color32 = Color32::from_rgb(216, 222, 233);
const BLUE: Color32 = Color32::from_rgb(129, 161, 193);
const YELLOW: Color32 = Color32::from_rgb(235, 203, 139);
const MAGENTA: Color32 = Color32::from_rgb(180, 142, 173);
const CYAN: Color32 = Color32::from_rgb(136, 192, 208);
const BAR: Color32 = Color32::from_rgb(76, 86, 106);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Style {
  pub foreground: Color32,
  pub background: Option<Color32>,
  pub bold: bool,
  pub underline: bool,
  pub italic: bool,
}

impl Default for Style {
  fn default() -> Self {
    Self {
      foreground: FOREGROUND,
      background: None,
      bold: false,
      underline: false,
      italic: false,
    }
  }
}

impl Style {
  pub fn apply(&mut self, role: Role) {
    let patch = match role.style() {
      StyleChange::Reset(patch) => {
        *self = Self::default();
        patch
      }
      StyleChange::Patch(patch) => patch,
    };
    match patch.foreground {
      ColorChange::Keep => {}
      ColorChange::Default => self.foreground = FOREGROUND,
      ColorChange::Set(tone) => self.foreground = resolve(tone),
    }
    match patch.background {
      ColorChange::Keep => {}
      ColorChange::Default => self.background = None,
      ColorChange::Set(tone) => self.background = Some(resolve(tone)),
    }
    for attribute in patch.attributes {
      match attribute {
        Attribute::Bold => self.bold = true,
        Attribute::Dim => self.foreground = self.foreground.gamma_multiply(0.5),
        Attribute::Italic => self.italic = true,
        Attribute::Underline => self.underline = true,
        Attribute::Inverse => {
          let foreground = self.foreground;
          self.foreground = self.background.unwrap_or(BACKGROUND);
          self.background = Some(foreground);
        }
      }
    }
  }
}

const fn resolve(tone: Tone) -> Color32 {
  match tone {
    Tone::Black => Color32::from_rgb(32, 36, 47),
    Tone::White => Color32::from_rgb(229, 233, 240),
    Tone::BrightWhite => Color32::WHITE,
    Tone::Blue => BLUE,
    Tone::Magenta => MAGENTA,
    Tone::Yellow => YELLOW,
    Tone::Cyan | Tone::ActiveBar => CYAN,
    Tone::Bar => BAR,
    Tone::Error => Color32::from_rgb(191, 97, 106),
    Tone::PreviewBorder => Color32::from_rgb(60, 120, 230),
    Tone::PreviewText => Color32::from_gray(20),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_model::style::Color;

  /// Role, RGBA foreground and background, then bold, underline,
  /// and italic flags.
  type Case = (Role, u32, Option<u32>, &'static str);

  const CASES: [Case; 35] = [
    (Role::Reset, 0xd8de_e9ff, None, ""),
    (Role::Bar, 0xd8de_e9ff, Some(0x4c56_6aff), ""),
    (Role::Selected, 0xd8de_e9ff, Some(0x4c56_6aff), "b"),
    (Role::StatusFile, 0x2024_2fff, Some(0x88c0_d0ff), "b"),
    (Role::TabInactive, 0xe5e9_f0ff, Some(0x2024_2fff), ""),
    (Role::Current, 0x81a1_c1ff, None, "b"),
    (Role::Hover, 0xd8de_e9ff, None, "u"),
    (Role::HoverDim, 0x6c6f_7580, None, "u"),
    (Role::BarHover, 0xd8de_e9ff, Some(0x4c56_6aff), "u"),
    (Role::Heading, 0x81a1_c1ff, None, "b"),
    (Role::Key, 0xebcb_8bff, None, ""),
    (Role::Mode, 0x88c0_d0ff, None, ""),
    (Role::Argument, 0xb48e_adff, None, ""),
    (Role::Muted, 0x4c56_6aff, None, ""),
    (Role::Dim, 0x6c6f_7580, None, ""),
    (Role::StatusSection, 0xd8de_e9ff, Some(0x4c56_6aff), "i"),
    (Role::Error, 0xbf61_6aff, None, "b"),
    (Role::TextLanding, 0xd8de_e9ff, Some(0xebcb_8bff), ""),
    (Role::TextCursor, 0x181b_24ff, Some(0x81a1_c1ff), ""),
    (Role::TextSelection, 0x181b_24ff, Some(0xb48e_adff), ""),
    (Role::TextMatch, 0x181b_24ff, Some(0xebcb_8bff), ""),
    (Role::HintTyped, 0x2024_2fff, Some(0xebcb_8bff), "b"),
    (Role::Hint, 0x2024_2fff, Some(0xebcb_8bff), ""),
    (Role::HintPickedTyped, 0xffff_ffff, Some(0x81a1_c1ff), "b"),
    (Role::HintPicked, 0xffff_ffff, Some(0x81a1_c1ff), ""),
    (Role::TextLink, 0x88c0_d0ff, None, "u"),
    (Role::PreviewBorder, 0x3c78_e6ff, Some(0xffff_ffff), ""),
    (Role::PreviewText, 0x1414_14ff, Some(0xffff_ffff), ""),
    (Role::Badge(Color::Blue), 0x181b_24ff, Some(0x81a1_c1ff), "b"),
    (Role::Badge(Color::Magenta), 0x181b_24ff, Some(0xb48e_adff), "b"),
    (Role::Badge(Color::Yellow), 0x181b_24ff, Some(0xebcb_8bff), "b"),
    (Role::Badge(Color::Cyan), 0x181b_24ff, Some(0x88c0_d0ff), "b"),
    (Role::TextCursorline, 0xd8de_e9ff, Some(0x4c56_6aff), ""),
    (Role::TabActive, 0x2024_2fff, Some(0x88c0_d0ff), "b"),
    (Role::File, 0x81a1_c1ff, None, "b"),
  ];

  #[test]
  fn roles_keep_resolved_styles() {
    let color = |rgba: u32| {
      let [r, g, b, a] = rgba.to_be_bytes();
      Color32::from_rgba_premultiplied(r, g, b, a)
    };
    for &(role, foreground, background, flags) in &CASES {
      let expected = Style {
        foreground: color(foreground),
        background: background.map(color),
        bold: flags.contains('b'),
        underline: flags.contains('u'),
        italic: flags.contains('i'),
      };
      for &(previous, ..) in &CASES {
        let mut actual = Style::default();
        actual.apply(previous);
        let mut composed = expected;
        if matches!(
          role,
          Role::Heading
            | Role::File
            | Role::Key
            | Role::Mode
            | Role::Argument
            | Role::Muted
            | Role::Error
        ) {
          composed.background = actual.background;
          composed.bold |= actual.bold;
          composed.underline = actual.underline;
          composed.italic = actual.italic;
        }
        actual.apply(role);
        assert_eq!(actual, composed, "{previous:?} then {role:?}");
      }
    }
  }
}
