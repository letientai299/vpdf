pub use vpdf_model::style::{Color, Role};

/// Thumbnail rings: selected, hovered.
pub const RING: [u8; 4] = [60, 120, 230, 90];
pub const RING_HOVER: [u8; 4] = [150, 150, 150, 70];
pub const LANDING: [u8; 4] = [255, 190, 30, 65];
pub const CURSOR: [u8; 4] = [25, 100, 255, 120];
pub const SELECTION: [u8; 4] = [70, 130, 255, 85];
pub const MATCH: [u8; 4] = [255, 190, 30, 90];
pub const PANE_VEIL: [u8; 4] = [0, 0, 0, 70];
/// Dims the page around a capture region.
pub const REGION_VEIL: [u8; 4] = [0, 0, 0, 100];
pub const REGION_EDGE: [u8; 4] = [60, 120, 230, 220];
pub const CURSORLINE: [u8; 4] = [70, 130, 255, 30];
pub const HINT_FILL: [u8; 4] = [255, 205, 60, 255];
/// Page-view fills under badge and bar cells.
pub const BADGE_FILL: [u8; 4] = [60, 120, 230, 255];
pub const BAR_FILL: [u8; 4] = [70, 70, 70, 255];
/// Scrollbar track, thumb, and ticks.
pub const TRACK: [u8; 4] = [128, 128, 128, 50];
pub const THUMB: [u8; 4] = [110, 110, 110, 190];
pub const HEADING_TICK: [u8; 4] = [60, 120, 230, 230];
pub const MATCH_TICK: [u8; 4] = [255, 190, 30, 240];
/// The hovered link's tint.
pub const LINK: [u8; 4] = [40, 180, 220, 70];
pub const PREVIEW_FILL: [u8; 4] = [255; 4];
/// Gray gaps between preview pages.
pub const PREVIEW_GAP: u8 = 200;
