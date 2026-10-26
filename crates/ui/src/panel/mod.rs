use self::recent::Shelf;
use crate::keys::{Action, Input, PanelKind};
use crate::layout::{Screen, screen_step};
use crate::panel::layout::Layout;
use crate::panel::search::RowSearch;
use crate::pointer::{WHEEL_ROWS, Wheel};
use crate::text::Position;
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;
use vpdf_model::document::{Annotation, Target};
use vpdf_model::input::{MouseButton, MouseEvent, MouseEventKind};
use vpdf_reader::motion::Motion;
use vpdf_reader::reader::JumpDescription;

#[derive(Clone, PartialEq)]
pub enum Destination {
  Position(Position),
  Jump {
    position: Position,
    description: Option<JumpDescription>,
  },
  Annotation {
    position: Position,
    annotation: Annotation,
  },
  Link(Target),
  File(PathBuf),
  /// A global mark, resolved by the session.
  Mark(char),
  None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Style {
  Plain,
  Muted,
  Heading,
  /// `keys: description`, with the keys highlighted.
  Key,
  ModeKey,
  CommandKey,
  /// `parent/file`, with the file name highlighted.
  Path,
  /// A file name, in magenta.
  Document,
  /// A search excerpt; the match's bytes in the label.
  Excerpt {
    start: usize,
    end: usize,
  },
}

/// What a row points at; picks its glyph.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RowKind {
  #[default]
  Plain,
  Figure,
  Table,
  /// A caption-like line in body text.
  Mention,
  Section,
  Search,
  Link,
  Note,
  Underline,
  Highlight,
}

#[derive(Clone)]
pub struct Row {
  pub label: String,
  /// The page, right-aligned; Jumps keeps its position here.
  pub detail: String,
  /// Accent text before the label, such as `Fig 1`.
  pub key: String,
  pub kind: RowKind,
  /// 0 for a group heading, 1 for its rows; Contents keeps its
  /// depths in its tree.
  pub depth: usize,
  pub style: Style,
  pub destination: Destination,
}

/// Narrowest dragged panel.
const MIN_WIDTH: u16 = 8;

pub struct Panel {
  kind: PanelKind,
  rows: Vec<Row>,
  projection: tree::Projection,
  tree_view: Option<outline::TreeView>,
  /// The panel's own groups; Contents keeps its folds in the reader.
  folds: Option<folds::Folds>,
  selected: usize,
  focused: bool,
  /// First visible row; moves only to keep the selection shown.
  top: usize,
  search: RowSearch,
  /// Shown after the row count, such as "inferred".
  note: &'static str,
  /// What the rows answer, such as the search query.
  topic: String,
  /// The row at the reading position.
  current: Option<usize>,
  /// Columns the last frame drew; presses beyond them miss the rows.
  columns: u16,
  height: u16,
  layout: Option<Layout>,
  follow_pending: bool,
  /// Cards for recent documents; a reader pane shows them over its page.
  shelf: Option<Shelf>,
  recent: selection::Selection,
  picker: crate::home::Picker,
  /// What the pointer rests on.
  pointing: Pointing,
  /// The hovered row's label when drawn clipped.
  hover_title: Option<String>,
  /// The row a click just followed.
  clicked: Option<usize>,
}

/// Where `old[index]` sits in `new`: the equal row nearest it, as rows
/// shift.
fn moved_row(old: &[Row], index: usize, new: &[Row]) -> Option<usize> {
  let row = old.get(index)?;
  let same = |other: &Row| {
    if let (Destination::File(path), Destination::File(other)) =
      (&row.destination, &other.destination)
    {
      return path == other;
    }
    other.label == row.label
      && other.detail == row.detail
      && other.destination == row.destination
  };
  let found = new.iter().enumerate().filter(|(_, other)| same(other));
  found.map(|(at, _)| at).min_by_key(|at| at.abs_diff(index))
}

/// Where the pointer rests in a panel.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Pointing {
  #[default]
  Away,
  Row(usize),
  Control(Control),
  /// The recent card.
  Card,
}

impl Pointing {
  pub fn row(self) -> Option<usize> {
    match self {
      Self::Row(index) | Self::Control(Control::Branch(index)) => Some(index),
      Self::Away | Self::Card | Self::Control(_) => None,
    }
  }
}

pub use info::{DocumentInfo, Info, document_lines, metadata_lines};
pub use jumps::key as jump_key;
pub use rows::key_width;
mod info;

#[cfg(test)]
mod annotations_tests;
#[cfg(test)]
mod focus_tests;
#[cfg(test)]
mod follow_tests;
#[cfg(test)]
mod projection_tests;
#[cfg(test)]
mod results_tests;
#[cfg(test)]
mod wrap_tests;

mod content;
mod empty;
pub(crate) mod folds;
#[cfg(test)]
mod folds_tests;
mod header;
mod outline;
pub use outline::Control;
mod follow;
#[cfg(test)]
mod outline_tests;
pub mod tree;
pub use follow::{Activation, Focus, Follow, Press};
mod input;
mod selection;
#[cfg(test)]
mod selection_tests;
pub use input::Event;
pub(crate) mod layout;
#[cfg(test)]
mod layout_tests;
mod rows;
mod search;
#[cfg(test)]
mod tests;
mod view;
pub use view::page_space;

mod jumps;
pub use layout::wrap;

pub mod recent;
