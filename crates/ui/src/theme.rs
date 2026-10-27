pub use vpdf_model::style::{Color, Role};
pub use vpdf_model::theme::{Overlay, Paint};

/// Thumbnail rings: selected, hovered.
pub const RING: Paint = Paint::Overlay(Overlay::Ring);
pub const RING_HOVER: Paint = Paint::Overlay(Overlay::RingHover);
pub const LANDING: Paint = Paint::Overlay(Overlay::Landing);
pub const CURSOR: Paint = Paint::Overlay(Overlay::Cursor);
pub const SELECTION: Paint = Paint::Overlay(Overlay::Selection);
pub const MATCH: Paint = Paint::Overlay(Overlay::Match);
pub const PANE_VEIL: Paint = Paint::Overlay(Overlay::PaneVeil);
/// Dims the page around a capture region.
pub const REGION_VEIL: Paint = Paint::Overlay(Overlay::RegionVeil);
pub const REGION_EDGE: Paint = Paint::Overlay(Overlay::RegionEdge);
pub const CURSORLINE: Paint = Paint::Overlay(Overlay::Cursorline);
pub const HINT_FILL: Paint = Paint::Overlay(Overlay::Hint);
/// Page-view fills under badge and bar cells.
pub const BADGE_FILL: Paint = Paint::Overlay(Overlay::Badge);
pub const BAR_FILL: Paint = Paint::Overlay(Overlay::Bar);
/// Scrollbar track, thumb, and ticks.
pub const TRACK: Paint = Paint::Overlay(Overlay::Track);
pub const THUMB: Paint = Paint::Overlay(Overlay::Thumb);
pub const HEADING_TICK: Paint = Paint::Overlay(Overlay::HeadingTick);
pub const MATCH_TICK: Paint = Paint::Overlay(Overlay::MatchTick);
/// The hovered link's tint.
pub const LINK: Paint = Paint::Overlay(Overlay::Link);
pub const PREVIEW_FILL: Paint = Paint::Overlay(Overlay::Preview);
