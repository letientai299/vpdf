//! SGR styles from the terminal's 16-color palette, so the chrome follows
//! the shell theme instead of fixed RGB values.

pub const RESET: &str = "\x1b[0m";
pub const BAR: &str = "\x1b[0;100m";
pub const SELECTED: &str = "\x1b[0;1;100m";
pub const TAB: &str = "\x1b[0;1;34;100m";
pub const CURRENT: &str = "\x1b[0;1;34m";
pub const HEADING: &str = "\x1b[1;34m";
pub const KEY: &str = "\x1b[33m";
/// File names in path lists.
pub const FILE: &str = "\x1b[1;34m";
pub const MUTED: &str = "\x1b[90m";
/// Faint text for the side that does not have focus.
pub const DIM: &str = "\x1b[0;2m";
/// Muted text that keeps the status bar background.
pub const MUTED_BAR: &str = "\x1b[0;3;100m";
pub const ERROR: &str = "\x1b[1;31m";
pub const TEXT_CURSOR: &str = "\x1b[0;7;34m";
pub const TEXT_SELECTION: &str = "\x1b[0;7;35m";
pub const TEXT_MATCH: &str = "\x1b[0;7;33m";

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
