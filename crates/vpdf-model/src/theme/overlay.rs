use super::Theme;

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
  pub fn resolve(self, theme: Theme) -> [u8; 4] {
    match self {
      Self::Rgba(rgba) => rgba,
      Self::Overlay(overlay) => overlay.resolve(theme),
    }
  }
}

impl Overlay {
  pub fn resolve(self, theme: Theme) -> [u8; 4] {
    if theme == Theme::Default {
      return self.default_color();
    }
    let p = theme.palette();
    let rgb = match self {
      Self::Ring
      | Self::Cursor
      | Self::RegionEdge
      | Self::Badge
      | Self::HeadingTick => p.blue,
      Self::RingHover | Self::Track | Self::Thumb => p.muted,
      Self::Landing | Self::Match | Self::Hint | Self::MatchTick => p.yellow,
      Self::PaneVeil | Self::RegionVeil | Self::Preview => p.background,
      Self::Selection => p.magenta,
      Self::Cursorline => p.selection,
      Self::Bar => p.surface,
      Self::Link => p.cyan,
    };
    [rgb[0], rgb[1], rgb[2], self.default_color()[3]]
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
