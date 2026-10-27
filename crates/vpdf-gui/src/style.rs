use eframe::egui::Color32;
use vpdf_model::style::{Attribute, ColorChange, Role, StyleChange};
use vpdf_model::theme::Colors;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Cue {
  #[default]
  None,
  Selection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Content {
  Text,
  Track,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Style {
  pub foreground: Color32,
  pub background: Option<Color32>,
  pub bold: bool,
  pub underline: bool,
  pub italic: bool,
  cue: Cue,
  content: Content,
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
      cue: Cue::None,
      content: Content::Text,
    }
  }

  pub fn accommodate(
    &mut self,
    display: crate::preferences::Display,
    theme: Colors,
  ) {
    self.underline |= self.cue == Cue::Selection;
    if !display.contrast || self.content == Content::Track {
      return;
    }
    let fill =
      self.background.unwrap_or_else(|| rgb(theme.palette().background));
    let bytes = |color: Color32| {
      let [r, g, b, _] = color.to_array();
      [r, g, b]
    };
    if vpdf_model::theme::contrast(bytes(self.foreground), bytes(fill)) < 7.0 {
      self.foreground = [Color32::BLACK, Color32::WHITE]
        .into_iter()
        .max_by(|a, b| {
          vpdf_model::theme::contrast(bytes(*a), bytes(fill))
            .total_cmp(&vpdf_model::theme::contrast(bytes(*b), bytes(fill)))
        })
        .unwrap();
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
    self.content = match role {
      Role::Track => Content::Track,
      _ => Content::Text,
    };
    if matches!(
      role,
      Role::Selected
        | Role::TabActive
        | Role::Current
        | Role::TextCursor
        | Role::TextSelection
    ) {
      self.cue = Cue::Selection;
    }
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
        Attribute::Dim => self.foreground = rgb(theme.palette().muted),
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
  fn accommodations_keep_theme_and_cues() {
    for (_, palette) in shipped() {
      let theme = Colors::Palette(palette);
      for role in
        [Role::Selected, Role::TabActive, Role::Current, Role::TextSelection]
      {
        let mut style = Style::new(theme);
        style.apply(role, theme);
        let original = style;
        style.accommodate(crate::preferences::Display::default(), theme);
        assert!(style.underline);
        assert_eq!(style.foreground, original.foreground);
        assert_eq!(style.background, original.background);
        style.accommodate(
          crate::preferences::Display { contrast: true, ..Default::default() },
          theme,
        );
        let bytes = |color: Color32| {
          let [r, g, b, _] = color.to_array();
          [r, g, b]
        };
        let fill = style.background.unwrap_or_else(|| rgb(palette.background));
        assert!(
          vpdf_model::theme::contrast(bytes(style.foreground), bytes(fill))
            >= 4.5
        );
      }
    }
  }

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
        (Role::Selected, p.foreground, Some(p.selection_bg())),
        (Role::TextCursorline, p.foreground, Some(p.cursorline)),
        (Role::Current, p.accent, None),
        (Role::Dim, p.muted, None),
        (Role::HoverDim, p.muted, None),
        (Role::StatusFile, p.background, Some(p.foreground)),
        (Role::TabActive, p.background, Some(p.foreground)),
        (Role::TabInactive, p.muted, Some(p.background)),
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

  fn shipped() -> Vec<(String, vpdf_model::theme::Palette)> {
    let folder =
      std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../etc/themes");
    let mut palettes: Vec<_> = std::fs::read_dir(folder)
      .unwrap()
      .map(|entry| entry.unwrap().path())
      .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
      .map(|path| {
        let text = std::fs::read_to_string(&path).unwrap();
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        (name, vpdf_model::theme::Palette::parse(&text).unwrap())
      })
      .collect();
    palettes.sort_by(|a, b| a.0.cmp(&b.0));
    palettes
  }

  /// Text and fill after applying `roles`.
  fn resolve(theme: Colors, roles: &[Role]) -> ([u8; 3], [u8; 3]) {
    let mut style = Style::new(theme);
    for &role in roles {
      style.apply(role, theme);
    }
    let fill = style.background.map_or(theme.palette().background, |fill| {
      let [r, g, b, _] = fill.to_array();
      [r, g, b]
    });
    let [r, g, b, _] = style.foreground.to_array();
    ([r, g, b], fill)
  }

  #[test]
  fn shipped_roles_are_readable() {
    use vpdf_model::style::Color;
    use vpdf_model::theme::{TEXT_CONTRAST, contrast};
    let sequences: &[&[Role]] = &[
      &[Role::Reset],
      &[Role::Reset, Role::Muted],
      &[Role::Dim, Role::Muted],
      &[Role::Hover, Role::Muted],
      &[Role::HoverDim, Role::Muted],
      &[Role::Reset, Role::Heading],
      &[Role::Reset, Role::Key],
      &[Role::Reset, Role::Mode],
      &[Role::Bar],
      &[Role::Bar, Role::Key],
      &[Role::Bar, Role::Error],
      &[Role::Selected],
      &[Role::Selected, Role::File],
      &[Role::Current],
      &[Role::TextCursorline],
      &[Role::TextCursor],
      &[Role::TextSelection],
      &[Role::TextMatch],
      &[Role::TextLink],
      &[Role::StatusFile],
      &[Role::TabInactive],
      &[Role::Hint],
      &[Role::HintPicked],
      &[Role::Chosen],
      &[Role::Badge(Color::Blue)],
      &[Role::Badge(Color::Cyan)],
      &[Role::Badge(Color::Magenta)],
      &[Role::Badge(Color::Yellow)],
    ];
    for (name, palette) in shipped() {
      let theme = Colors::Palette(palette);
      for roles in sequences {
        let (text, fill) = resolve(theme, roles);
        let ratio = contrast(text, fill);
        assert!(ratio >= TEXT_CONTRAST, "{name}: {roles:?} {ratio:.2}");
      }
      // Active tab needs 3:1.
      let tab = resolve(theme, &[Role::TabActive]).1;
      for other in [Role::Bar, Role::TabInactive] {
        let ratio = contrast(tab, resolve(theme, &[other]).1);
        assert!(ratio >= 3.0, "{name}: tab vs {other:?} {ratio:.2}");
      }
    }
  }

  #[test]
  fn track_preserves_selected_fill() {
    for palette in
      [vpdf_model::theme::Palette::light(), vpdf_model::theme::Palette::dark()]
    {
      let theme = Colors::Palette(palette);
      let mut style = Style::new(theme);
      style.apply(Role::Selected, theme);
      style.apply(Role::Track, theme);
      assert_eq!(style.background, Some(rgb(palette.selection_bg())));
      assert_eq!(style.foreground, rgb(palette.track()));
      assert!(style.bold);
      style.accommodate(
        crate::preferences::Display { contrast: true, ..Default::default() },
        theme,
      );
      assert_eq!(style.foreground, rgb(palette.track()));
      assert!(style.underline);
      style.apply(Role::Muted, theme);
      assert_eq!(style.foreground, rgb(palette.muted));
      assert_eq!(style.background, Some(rgb(palette.selection_bg())));
      assert!(
        vpdf_model::theme::contrast(palette.link, palette.track()) >= 3.0
      );
    }
  }

  #[test]
  fn muted_keeps_row_attributes() {
    for (name, palette) in shipped() {
      let theme = Colors::Palette(palette);
      for base in [Role::Reset, Role::Dim, Role::Hover, Role::HoverDim] {
        let mut style = Style::new(theme);
        style.apply(base, theme);
        let expected = Style { foreground: rgb(palette.muted), ..style };
        style.apply(Role::Muted, theme);
        assert_eq!(style, expected, "{name}: {base:?}");
        assert_eq!(
          style.underline,
          matches!(base, Role::Hover | Role::HoverDim),
          "{name}: {base:?}"
        );
      }
    }
  }

  #[test]
  fn mode_fills_color_badges() {
    use vpdf_model::style::Color;
    use vpdf_model::theme::Palette;
    let dark = include_str!("../../../etc/themes/dark.toml");
    let palette = Palette::parse(&format!(
      "{dark}\n[mode]\nnormal = \"#ff0000\"\nvisual = \"#101010\"\n"
    ))
    .unwrap();
    let mut gray = palette;
    gray.accent = [0x33; 3];
    // Chosen items keep `accent`, not `[mode] normal`.
    for (theme, role, foreground, background) in [
      (palette, Role::Badge(Color::Magenta), [255; 3], [16; 3]),
      (palette, Role::Badge(Color::Blue), [0; 3], [255, 0, 0]),
      (palette, Role::Chosen, palette.background, palette.accent),
      (gray, Role::Chosen, [255; 3], [0x33; 3]),
    ] {
      let theme = Colors::Palette(theme);
      let mut style = Style::new(theme);
      style.apply(role, theme);
      assert_eq!(style.foreground, rgb(foreground), "{role:?}");
      assert_eq!(style.background, Some(rgb(background)), "{role:?}");
      assert!(style.bold, "{role:?}");
    }
  }
}
