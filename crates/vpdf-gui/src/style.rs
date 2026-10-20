use eframe::egui::Color32;
use vpdf_model::style::{Color, Role};

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
    if self.partial(role) {
      return;
    }
    *self = Self::default();
    match role {
      Role::Bar | Role::TextCursorline => self.background = Some(BAR),
      Role::Selected => {
        self.background = Some(BAR);
        self.bold = true;
      }
      Role::StatusFile | Role::TabActive => {
        self.background = Some(CYAN);
        self.foreground = Color32::from_rgb(32, 36, 47);
        self.bold = true;
      }
      Role::TabInactive => {
        self.background = Some(Color32::from_rgb(32, 36, 47));
        self.foreground = Color32::from_rgb(229, 233, 240);
      }
      Role::Current => {
        self.foreground = BLUE;
        self.bold = true;
      }
      Role::Hover | Role::HoverDim | Role::TextLink => {
        self.underline = true;
        if role == Role::TextLink {
          self.foreground = CYAN;
        }
        if role == Role::HoverDim {
          self.foreground = FOREGROUND.gamma_multiply(0.5);
        }
      }
      Role::BarHover => {
        self.background = Some(BAR);
        self.underline = true;
      }
      Role::Dim => self.foreground = FOREGROUND.gamma_multiply(0.5),
      Role::StatusSection => {
        self.background = Some(BAR);
        self.italic = true;
      }
      Role::TextLanding => self.background = Some(YELLOW),
      Role::TextCursor
      | Role::TextSelection
      | Role::TextMatch
      | Role::Badge(_) => {
        self.background = Some(match role {
          Role::TextSelection | Role::Badge(Color::Magenta) => MAGENTA,
          Role::TextMatch | Role::Badge(Color::Yellow) => YELLOW,
          Role::Badge(Color::Cyan) => CYAN,
          _ => BLUE,
        });
        self.foreground = BACKGROUND;
        self.bold = matches!(role, Role::Badge(_));
      }
      Role::Hint | Role::HintTyped => {
        self.foreground = Color32::from_rgb(32, 36, 47);
        self.background = Some(YELLOW);
        self.bold = role == Role::HintTyped;
      }
      Role::HintPicked | Role::HintPickedTyped => {
        self.foreground = Color32::WHITE;
        self.background = Some(BLUE);
        self.bold = role == Role::HintPickedTyped;
      }
      Role::PreviewBorder => {
        self.foreground = Color32::from_rgb(60, 120, 230);
        self.background = Some(Color32::WHITE);
      }
      Role::PreviewText => {
        self.foreground = Color32::from_gray(20);
        self.background = Some(Color32::WHITE);
      }
      _ => {}
    }
  }
  fn partial(&mut self, role: Role) -> bool {
    match role {
      Role::Heading | Role::File => {
        self.foreground = BLUE;
        self.bold = true;
      }
      Role::Key => {
        self.foreground = YELLOW;
      }
      Role::Mode => {
        self.foreground = CYAN;
      }
      Role::Argument => {
        self.foreground = MAGENTA;
      }
      Role::Muted => {
        self.foreground = BAR;
      }
      Role::Error => {
        self.foreground = Color32::from_rgb(191, 97, 106);
        self.bold = true;
      }
      _ => return false,
    }
    true
  }
}
