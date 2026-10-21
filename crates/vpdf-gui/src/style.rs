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

  type Case = (Role, [u8; 4], Option<[u8; 4]>, bool, bool, bool);

  const CASES: [Case; 35] = [
    (Role::Reset, [216, 222, 233, 255], None, false, false, false),
    (
      Role::Bar,
      [216, 222, 233, 255],
      Some([76, 86, 106, 255]),
      false,
      false,
      false,
    ),
    (
      Role::Selected,
      [216, 222, 233, 255],
      Some([76, 86, 106, 255]),
      true,
      false,
      false,
    ),
    (
      Role::StatusFile,
      [32, 36, 47, 255],
      Some([136, 192, 208, 255]),
      true,
      false,
      false,
    ),
    (
      Role::TabInactive,
      [229, 233, 240, 255],
      Some([32, 36, 47, 255]),
      false,
      false,
      false,
    ),
    (Role::Current, [129, 161, 193, 255], None, true, false, false),
    (Role::Hover, [216, 222, 233, 255], None, false, true, false),
    (Role::HoverDim, [108, 111, 117, 128], None, false, true, false),
    (
      Role::BarHover,
      [216, 222, 233, 255],
      Some([76, 86, 106, 255]),
      false,
      true,
      false,
    ),
    (Role::Heading, [129, 161, 193, 255], None, true, false, false),
    (Role::Key, [235, 203, 139, 255], None, false, false, false),
    (Role::Mode, [136, 192, 208, 255], None, false, false, false),
    (Role::Argument, [180, 142, 173, 255], None, false, false, false),
    (Role::Muted, [76, 86, 106, 255], None, false, false, false),
    (Role::Dim, [108, 111, 117, 128], None, false, false, false),
    (
      Role::StatusSection,
      [216, 222, 233, 255],
      Some([76, 86, 106, 255]),
      false,
      false,
      true,
    ),
    (Role::Error, [191, 97, 106, 255], None, true, false, false),
    (
      Role::TextLanding,
      [216, 222, 233, 255],
      Some([235, 203, 139, 255]),
      false,
      false,
      false,
    ),
    (
      Role::TextCursor,
      [24, 27, 36, 255],
      Some([129, 161, 193, 255]),
      false,
      false,
      false,
    ),
    (
      Role::TextSelection,
      [24, 27, 36, 255],
      Some([180, 142, 173, 255]),
      false,
      false,
      false,
    ),
    (
      Role::TextMatch,
      [24, 27, 36, 255],
      Some([235, 203, 139, 255]),
      false,
      false,
      false,
    ),
    (
      Role::HintTyped,
      [32, 36, 47, 255],
      Some([235, 203, 139, 255]),
      true,
      false,
      false,
    ),
    (
      Role::Hint,
      [32, 36, 47, 255],
      Some([235, 203, 139, 255]),
      false,
      false,
      false,
    ),
    (
      Role::HintPickedTyped,
      [255, 255, 255, 255],
      Some([129, 161, 193, 255]),
      true,
      false,
      false,
    ),
    (
      Role::HintPicked,
      [255, 255, 255, 255],
      Some([129, 161, 193, 255]),
      false,
      false,
      false,
    ),
    (Role::TextLink, [136, 192, 208, 255], None, false, true, false),
    (
      Role::PreviewBorder,
      [60, 120, 230, 255],
      Some([255, 255, 255, 255]),
      false,
      false,
      false,
    ),
    (
      Role::PreviewText,
      [20, 20, 20, 255],
      Some([255, 255, 255, 255]),
      false,
      false,
      false,
    ),
    (
      Role::Badge(Color::Blue),
      [24, 27, 36, 255],
      Some([129, 161, 193, 255]),
      true,
      false,
      false,
    ),
    (
      Role::Badge(Color::Magenta),
      [24, 27, 36, 255],
      Some([180, 142, 173, 255]),
      true,
      false,
      false,
    ),
    (
      Role::Badge(Color::Yellow),
      [24, 27, 36, 255],
      Some([235, 203, 139, 255]),
      true,
      false,
      false,
    ),
    (
      Role::Badge(Color::Cyan),
      [24, 27, 36, 255],
      Some([136, 192, 208, 255]),
      true,
      false,
      false,
    ),
    (
      Role::TextCursorline,
      [216, 222, 233, 255],
      Some([76, 86, 106, 255]),
      false,
      false,
      false,
    ),
    (
      Role::TabActive,
      [32, 36, 47, 255],
      Some([136, 192, 208, 255]),
      true,
      false,
      false,
    ),
    (Role::File, [129, 161, 193, 255], None, true, false, false),
  ];

  #[test]
  fn roles_keep_resolved_styles() {
    for &(role, foreground, background, bold, underline, italic) in &CASES {
      let expected = Style {
        foreground: Color32::from_rgba_premultiplied(
          foreground[0],
          foreground[1],
          foreground[2],
          foreground[3],
        ),
        background: background
          .map(|c| Color32::from_rgba_premultiplied(c[0], c[1], c[2], c[3])),
        bold,
        underline,
        italic,
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
