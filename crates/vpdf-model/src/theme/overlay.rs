use super::{Colors, Opacity};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Overlay {
  Ring,
  RingHover,
  Landing,
  Cursor,
  Selection,
  Match,
  PaneVeil,
  Inactive,
  TextCursor,
  RegionVeil,
  RegionEdge,
  Cursorline,
  Hint,
  Badge,
  Bar,
  Track,
  Thumb,
  HeadingTick,
  MatchTick,
  Link,
  LinkDecoration,
  Preview,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Paint {
  Rgba([u8; 4]),
  Overlay(Overlay),
}

impl From<[u8; 4]> for Paint {
  fn from(rgba: [u8; 4]) -> Self {
    Self::Rgba(rgba)
  }
}

impl Paint {
  pub fn resolve(self, theme: Colors, opacity: Opacity) -> [u8; 4] {
    match self {
      Self::Rgba(rgba) => rgba,
      Self::Overlay(overlay) => overlay.resolve(theme, opacity),
    }
  }
}

impl Overlay {
  pub fn resolve(self, theme: Colors, opacity: Opacity) -> [u8; 4] {
    let p = theme.palette();
    let rgb = match self {
      Self::Ring
      | Self::Cursor
      | Self::RegionEdge
      | Self::Badge
      | Self::HeadingTick => p.accent,
      Self::RingHover | Self::Thumb => p.muted,
      Self::Track => p.track(),
      Self::Landing | Self::Match | Self::Hint | Self::MatchTick => p.r#match,
      Self::PaneVeil | Self::Inactive | Self::RegionVeil | Self::Preview => {
        p.background
      }
      Self::Selection => p.highlight,
      Self::TextCursor => p.foreground,
      Self::Cursorline => p.cursorline,
      Self::Bar => p.surface,
      Self::Link | Self::LinkDecoration => p.link,
    };
    [rgb[0], rgb[1], rgb[2], self.alpha(rgb, opacity)]
  }

  fn alpha(self, rgb: [u8; 3], opacity: Opacity) -> u8 {
    let (configured, primary) = match self {
      Self::Selection => (opacity.selection, Self::Selection),
      Self::Landing | Self::Match => (opacity.search, Self::Match),
      Self::Cursor => (opacity.cursor, Self::Cursor),
      Self::TextCursor => (opacity.text_cursor, Self::TextCursor),
      Self::Cursorline => (opacity.cursorline, Self::Cursorline),
      Self::Link => (opacity.link, Self::Link),
      Self::PaneVeil | Self::RegionVeil => (opacity.veil, Self::PaneVeil),
      Self::Ring | Self::RingHover => (opacity.ring, Self::Ring),
      Self::Track | Self::Thumb => (opacity.scrollbar, Self::Track),
      Self::Inactive => (opacity.inactive, Self::Inactive),
      _ => (None, self),
    };
    if let Some(value) = configured {
      let ratio =
        f64::from(self.default_alpha()) / f64::from(primary.default_alpha());
      let scaled = value.get() * ratio;
      return (scaled.min(1.0) * 255.0).round() as u8;
    }
    let alpha = self.default_alpha();
    match self {
      Self::Cursor
      | Self::Selection
      | Self::Match
      | Self::Landing
      | Self::PaneVeil
      | Self::RegionVeil
      | Self::Cursorline
      | Self::Link => {
        // Preserve 4.5:1 page contrast.
        // https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html
        let maximum =
          (255.0 * 0.825 / (1.0 + 3.5 * super::luminance(rgb))).floor() as u8;
        alpha.min(maximum)
      }
      _ => alpha,
    }
  }

  pub(super) const fn default_alpha(self) -> u8 {
    match self {
      Self::Ring | Self::Match => 90,
      Self::RingHover | Self::PaneVeil | Self::Link => 70,
      Self::Landing => 65,
      Self::Cursor => 120,
      Self::Selection => 85,
      Self::RegionVeil | Self::TextCursor => 100,
      Self::RegionEdge => 220,
      Self::Cursorline => 30,
      Self::Hint
      | Self::Badge
      | Self::Bar
      | Self::Preview
      | Self::LinkDecoration => 255,
      Self::Track => 50,
      Self::Thumb => 190,
      Self::HeadingTick => 230,
      Self::MatchTick => 240,
      Self::Inactive => 51,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::theme::{Palette, luminance};

  #[test]
  fn opacity_defaults_and_links() {
    let defaults = Opacity::default();
    let mut explicit = defaults;
    let mut half = defaults;
    for (key, value) in defaults.values() {
      *explicit.field(key).unwrap() = Some(value.try_into().unwrap());
      *half.field(key).unwrap() = Some(0.5.try_into().unwrap());
    }
    for (overlay, expected, scaled) in [
      (Overlay::Selection, 85, 128),
      (Overlay::Match, 90, 128),
      (Overlay::Landing, 65, 92),
      (Overlay::Cursor, 120, 128),
      (Overlay::TextCursor, 100, 128),
      (Overlay::Cursorline, 30, 128),
      (Overlay::Link, 70, 128),
      (Overlay::PaneVeil, 70, 128),
      (Overlay::RegionVeil, 100, 182),
      (Overlay::Ring, 90, 128),
      (Overlay::RingHover, 70, 99),
      (Overlay::Track, 50, 128),
      (Overlay::Thumb, 190, 255),
      (Overlay::Inactive, 51, 128),
      (Overlay::Hint, 255, 255),
      (Overlay::Badge, 255, 255),
      (Overlay::Bar, 255, 255),
      (Overlay::Preview, 255, 255),
      (Overlay::RegionEdge, 220, 220),
      (Overlay::HeadingTick, 230, 230),
      (Overlay::MatchTick, 240, 240),
    ] {
      assert_eq!(overlay.alpha([0; 3], defaults), expected, "{overlay:?}");
      assert_eq!(overlay.alpha([0; 3], explicit), expected, "{overlay:?}");
      assert_eq!(overlay.alpha([255; 3], half), scaled, "{overlay:?}");
    }
  }

  #[test]
  fn primary_opacity_rounds_directly() {
    for (key, overlay) in [
      ("selection", Overlay::Selection),
      ("search", Overlay::Match),
      ("cursor", Overlay::Cursor),
      ("text_cursor", Overlay::TextCursor),
      ("cursorline", Overlay::Cursorline),
      ("link", Overlay::Link),
      ("veil", Overlay::PaneVeil),
      ("ring", Overlay::Ring),
      ("scrollbar", Overlay::Track),
      ("inactive", Overlay::Inactive),
    ] {
      for byte in 0..255 {
        let value = (f64::from(byte) + 0.5) / 255.0;
        let mut opacity = Opacity::default();
        *opacity.field(key).unwrap() = Some(value.try_into().unwrap());
        assert_eq!(
          overlay.resolve(Colors::Terminal, opacity)[3],
          (value * 255.0).round() as u8,
          "{key}: {value}",
        );
      }
    }
  }

  #[test]
  fn explicit_opacity_bypasses_contrast() {
    let opacity = Opacity {
      selection: Some(1.0.try_into().unwrap()),
      ..Opacity::default()
    };
    assert_eq!(Overlay::Selection.alpha([255; 3], opacity), 255);
    assert_eq!(Overlay::Match.alpha([255; 3], opacity), 46);
    assert_eq!(Overlay::Selection.alpha([255; 3], Opacity::default()), 46);
  }

  #[test]
  fn terminal_overlays_use_palette() {
    let palette = Colors::Terminal.palette();
    for (overlay, rgb) in [
      (Overlay::Cursor, palette.accent),
      (Overlay::Selection, palette.highlight),
      (Overlay::Match, palette.r#match),
      (Overlay::PaneVeil, palette.background),
      (Overlay::Inactive, palette.background),
      (Overlay::Preview, palette.background),
      (Overlay::Link, palette.link),
      (Overlay::Track, palette.track()),
      (Overlay::Thumb, palette.muted),
    ] {
      assert_eq!(
        overlay.resolve(Colors::Terminal, Opacity::default())[..3],
        rgb
      );
      assert_eq!(
        overlay.resolve(Colors::Terminal, Opacity::default()),
        overlay.resolve(Colors::Palette(palette), Opacity::default())
      );
    }
  }

  #[test]
  fn scrollbar_colors_are_independent() {
    for mut palette in [Palette::light(), Palette::dark()] {
      palette.track = Some([0x12, 0x34, 0x56]);
      let theme = Colors::Palette(palette);
      assert_eq!(
        Overlay::Track.resolve(theme, Opacity::default()),
        [0x12, 0x34, 0x56, 50]
      );
      let [r, g, b] = palette.muted;
      assert_eq!(
        Overlay::Thumb.resolve(theme, Opacity::default()),
        [r, g, b, 190]
      );
    }
  }

  #[test]
  fn fills_preserve_page_contrast() {
    for mut palette in [Palette::light(), Palette::dark()] {
      for cursor in [palette.accent, [255; 3]] {
        palette.accent = cursor;
        for overlay in [
          Overlay::Cursor,
          Overlay::Selection,
          Overlay::Match,
          Overlay::Landing,
          Overlay::PaneVeil,
          Overlay::RegionVeil,
          Overlay::Cursorline,
          Overlay::Link,
        ] {
          let [r, g, b, a] =
            overlay.resolve(Colors::Palette(palette), Opacity::default());
          let alpha = f64::from(a) / 255.0;
          let ink = alpha * luminance([r, g, b]);
          let paper = 1.0 - alpha + ink;
          let ratio = (paper + 0.05) / (ink + 0.05);
          assert!(ratio >= crate::theme::TEXT_CONTRAST, "{overlay:?}: {ratio}");
        }
      }
    }
  }
}
