//! Window and tab commands that keys request and the session runs.

/// Commands on the panes of a tab.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Window {
  Split(crate::split::Axis),
  Focus(crate::split::Direction),
  Next,
  Previous,
  Last,
  Close,
  Only,
  Equalize,
  Resize(crate::split::Axis, i32),
  Extend(crate::split::Direction),
  Maximize(crate::split::Axis),
  Exchange,
  /// Rotates panes in the focused row or column; `true` moves forward.
  Rotate(bool),
  /// Splits above and lists recent documents to open there.
  New,
  /// Moves the pane to the full-height or full-width edge.
  Move(crate::split::Direction),
  ToTab,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TabMove {
  Next(usize),
  Previous(usize),
  Go(usize),
}
