use super::Colors;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Overlay {
  Ring,
  RingHover,
  Landing,
  Cursor,
  Selection,
  Match,
  PaneVeil,
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
  pub fn resolve(self, theme: Colors) -> [u8; 4] {
    match self {
      Self::Rgba(rgba) => rgba,
      Self::Overlay(overlay) => overlay.resolve(theme),
    }
  }
}

impl Overlay {
  pub fn resolve(self, theme: Colors) -> [u8; 4] {
    if theme == Colors::Terminal {
      return self.default_color();
    }
    let p = theme.palette();
    let rgb = match self {
      Self::Ring
      | Self::Cursor
      | Self::RegionEdge
      | Self::Badge
      | Self::HeadingTick => p.accent,
      Self::RingHover | Self::Track | Self::Thumb => p.muted,
      Self::Landing | Self::Match | Self::Hint | Self::MatchTick => p.r#match,
      Self::PaneVeil | Self::RegionVeil | Self::Preview => p.background,
      Self::Selection => p.highlight,
      Self::Cursorline => p.cursorline,
      Self::Bar => p.surface,
      Self::Link => p.link,
    };
    [rgb[0], rgb[1], rgb[2], self.alpha(rgb)]
  }

  fn alpha(self, rgb: [u8; 3]) -> u8 {
    let alpha = self.default_color()[3];
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

  const fn default_color(self) -> [u8; 4] {
    match self {
      Self::Ring => [60, 120, 230, 90],
      Self::RingHover => [150, 150, 150, 70],
      Self::Landing => [255, 190, 30, 65],
      Self::Cursor => [25, 100, 255, 120],
      Self::Selection => [70, 130, 255, 85],
      Self::Match => [255, 190, 30, 90],
      Self::PaneVeil => [0, 0, 0, 70],
      Self::RegionVeil => [0, 0, 0, 100],
      Self::RegionEdge => [60, 120, 230, 220],
      Self::Cursorline => [70, 130, 255, 30],
      Self::Hint => [255, 205, 60, 255],
      Self::Badge => [60, 120, 230, 255],
      Self::Bar => [70, 70, 70, 255],
      Self::Track => [128, 128, 128, 50],
      Self::Thumb => [110, 110, 110, 190],
      Self::HeadingTick => [60, 120, 230, 230],
      Self::MatchTick => [255, 190, 30, 240],
      Self::Link => [40, 180, 220, 70],
      Self::Preview => [255; 4],
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::theme::{Palette, luminance};

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
          let [r, g, b, a] = overlay.resolve(Colors::Palette(palette));
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
