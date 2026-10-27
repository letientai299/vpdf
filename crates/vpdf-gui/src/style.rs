use eframe::egui::Color32;
use vpdf_model::style::{Attribute, ColorChange, Role, StyleChange};
use vpdf_model::theme::Colors;

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
    Self::new(Colors::Terminal)
  }
}

pub fn rgb(color: [u8; 3]) -> Color32 {
  Color32::from_rgb(color[0], color[1], color[2])
}

impl Style {
  pub fn new(theme: Colors) -> Self {
    let palette = theme.palette();
    Self {
      foreground: rgb(palette.foreground),
      background: None,
      bold: false,
      underline: false,
      italic: false,
    }
  }

  pub fn apply(&mut self, role: Role, theme: Colors) {
    let theme = Colors::Palette(theme.palette());
    let patch = match theme.style(role) {
      StyleChange::Reset(patch) => {
        *self = Self::new(theme);
        patch
      }
      StyleChange::Patch(patch) => patch,
    };
    match patch.foreground {
      ColorChange::Keep => {}
      ColorChange::Default => self.foreground = rgb(theme.palette().foreground),
      ColorChange::Set(tone) => self.foreground = rgb(theme.color(tone, false)),
    }
    match patch.background {
      ColorChange::Keep => {}
      ColorChange::Default => self.background = Self::new(theme).background,
      ColorChange::Set(tone) => {
        self.background = Some(rgb(theme.color(tone, true)));
      }
    }
    for attribute in patch.attributes {
      match attribute {
        Attribute::Bold => self.bold = true,
        Attribute::Dim => self.foreground = self.foreground.gamma_multiply(0.5),
        Attribute::Italic => self.italic = true,
        Attribute::Underline => self.underline = true,
        Attribute::Inverse => {
          let foreground = self.foreground;
          self.foreground =
            self.background.unwrap_or_else(|| rgb(theme.palette().background));
          self.background = Some(foreground);
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn named_roles_resolve_palette() {
    for theme in [
      Colors::Palette(vpdf_model::theme::Palette::light()),
      Colors::Palette(vpdf_model::theme::Palette::dark()),
    ] {
      let p = theme.palette();
      let mut style = Style::new(theme);
      for (role, foreground, background) in [
        (Role::Reset, p.foreground, None),
        (Role::Bar, p.foreground, Some(p.surface)),
        (Role::TextCursorline, p.foreground, Some(p.cursorline)),
        (Role::Current, p.accent, None),
        (Role::Dim, p.muted, None),
        (Role::HoverDim, p.muted, None),
        (Role::StatusFile, p.background, Some(p.link)),
        (Role::TextSelection, p.background, Some(p.highlight)),
        (Role::PreviewText, p.foreground, Some(p.background)),
      ] {
        style.apply(role, theme);
        assert_eq!(style.foreground, rgb(foreground), "{theme:?}: {role:?}");
        assert_eq!(
          style.background,
          background.map(rgb),
          "{theme:?}: {role:?}"
        );
      }
    }
  }

  #[test]
  fn role_changes_reset_attributes() {
    for theme in
      [Colors::Terminal, Colors::Palette(vpdf_model::theme::Palette::light())]
    {
      let p = theme.palette();
      let mut style = Style::new(theme);
      style.apply(Role::Bar, theme);
      style.apply(Role::Heading, theme);
      assert!(style.bold);
      assert_eq!(style.background, Some(rgb(p.surface)));
      style.apply(Role::TextLink, theme);
      assert!(style.underline);
      assert!(!style.bold && !style.italic);
      assert_eq!(style.background, None);
      style.apply(Role::StatusSection, theme);
      assert!(style.italic);
      assert!(!style.bold && !style.underline);
      style.apply(Role::TextCursor, theme);
      assert_eq!(style.foreground, rgb(p.background));
      assert_eq!(style.background, Some(rgb(p.accent)));
      style.apply(Role::Reset, theme);
      assert_eq!(style, Style::new(theme));
    }
  }
}
