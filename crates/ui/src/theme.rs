//! SGR styles from the terminal's 16-color palette, so the chrome follows
//! the shell theme instead of fixed RGB values.

pub const RESET: &str = "\x1b[0m";
pub const BAR: &str = "\x1b[0;100m";
pub const SELECTED: &str = "\x1b[0;1;100m";
pub const STATUS_FILE: &str = "\x1b[0;1;30;104m";
pub const TAB_ACTIVE: &str = STATUS_FILE;
pub const TAB_INACTIVE: &str = "\x1b[0;37;40m";
pub const CURRENT: &str = "\x1b[0;1;34m";
/// A row under the pointer.
pub const HOVER: &str = "\x1b[0;4m";
pub const HOVER_DIM: &str = "\x1b[0;2;4m";
/// A pointed item on the bar.
pub const BAR_HOVER: &str = "\x1b[0;4;100m";
/// Thumbnail rings: selected, hovered.
pub const RING: [u8; 4] = [60, 120, 230, 90];
pub const RING_HOVER: [u8; 4] = [150, 150, 150, 70];
pub const HEADING: &str = "\x1b[1;34m";
pub const KEY: &str = "\x1b[33m";
pub const MODE: &str = "\x1b[36m";
pub const ARGUMENT: &str = "\x1b[35m";
/// File names in path lists.
pub const FILE: &str = "\x1b[1;34m";
pub const MUTED: &str = "\x1b[90m";
/// Faint text for the side that does not have focus.
pub const DIM: &str = "\x1b[0;2m";
/// Muted text that keeps the status bar background.
pub const STATUS_SECTION: &str = "\x1b[0;3;100m";
pub const ERROR: &str = "\x1b[1;31m";
pub const LANDING: [u8; 4] = [255, 190, 30, 65];
pub const TEXT_LANDING: &str = "\x1b[0;43m";
pub const CURSOR: [u8; 4] = [25, 100, 255, 120];
pub const SELECTION: [u8; 4] = [70, 130, 255, 85];
pub const MATCH: [u8; 4] = [255, 190, 30, 90];
pub const PANE_VEIL: [u8; 4] = [0, 0, 0, 70];
pub const CURSORLINE: [u8; 4] = [70, 130, 255, 30];
pub const TEXT_CURSORLINE: &str = "\x1b[0;100m";
pub const TEXT_CURSOR: &str = "\x1b[0;7;34m";
pub const TEXT_SELECTION: &str = "\x1b[0;7;35m";
pub const TEXT_MATCH: &str = "\x1b[0;7;33m";
/// Hint labels: typed, rest, fill.
pub const HINT_TYPED: &str = "\x1b[0;1;30;43m";
pub const HINT: &str = "\x1b[0;30;43m";
pub const HINT_FILL: [u8; 4] = [255, 205, 60, 255];
/// The label the pointer picked.
pub const HINT_PICKED_TYPED: &str = "\x1b[0;1;97;44m";
pub const HINT_PICKED: &str = "\x1b[0;97;44m";
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
/// The hovered link in text mode.
pub const TEXT_LINK: &str = "\x1b[0;4;36m";

#[derive(Clone, Copy)]
pub enum Color {
  Blue = 34,
  Magenta = 35,
  Yellow = 33,
  Cyan = 36,
}

/// Reversed palette color: the theme background on the accent.
pub fn badge(color: Color) -> String {
  format!("\x1b[0;1;7;{}m", color as u8)
}

/// Preview borders and text over page pixels.
pub const PREVIEW_BORDER: &str = "\x1b[0;38;2;60;120;230;48;2;255;255;255m";
pub const PREVIEW_TEXT: &str = "\x1b[0;38;2;20;20;20;48;2;255;255;255m";
pub const PREVIEW_FILL: [u8; 4] = [255; 4];
