use crate::Result;
use crate::app::{App, Request};
use crate::document::{Annotation, Heading, Target};
use crate::graphics::Graphics;
use crate::keys::{Action, Input, PanelKind};
use crate::layout::{Screen, screen_step};
use crate::panel_layout::Layout;
use crate::panel_search::RowSearch;
use crate::pointer::{WHEEL_ROWS, Wheel};
use crate::recent::Shelf;
use crate::state::Entry;
use crate::terminal;
use crate::text::{PageId, Position};
use crate::theme::{self, Color};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use std::io::Write;
use std::path::{Path, PathBuf};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
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
}

pub struct Row {
  pub label: String,
  pub detail: String,
  pub style: Style,
  pub destination: Destination,
}

/// Narrowest dragged panel.
const MIN_WIDTH: u16 = 8;

pub struct Panel {
  pub kind: PanelKind,
  pub rows: Vec<Row>,
  pub selected: usize,
  pub focused: bool,
  /// First visible row; moves only to keep the selection shown.
  pub top: usize,
  pub search: RowSearch,
  /// Shown after the row count, such as "inferred".
  pub note: &'static str,
  /// Columns the last frame drew; presses beyond them miss the rows.
  columns: u16,
  height: u16,
  layout: Option<Layout>,
  follow_pending: bool,
  /// Cards for recent documents; a reader pane shows them over its page.
  pub shelf: Option<Shelf>,
  /// What the pointer rests on.
  pub(crate) pointing: Pointing,
  /// The hovered row's label when drawn clipped.
  pub(crate) hover_title: Option<String>,
  /// The row a click just followed.
  pub(crate) clicked: Option<usize>,
}

/// Where `old[index]` sits in `new`: the equal row nearest it, as rows
/// shift.
fn moved_row(old: &[Row], index: usize, new: &[Row]) -> Option<usize> {
  let row = old.get(index)?;
  let same = |other: &Row| {
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
  /// The recent card.
  Card,
}

impl Pointing {
  pub fn row(self) -> Option<usize> {
    match self {
      Self::Row(index) => Some(index),
      Self::Away | Self::Card => None,
    }
  }
}

pub use rows::key_width;
pub use sections::{document_lines, metadata_lines, recent_rows, section};

#[cfg(test)]
mod focus_tests;
#[cfg(test)]
mod help_tests;

mod actions;
mod rows;
mod sections;
#[cfg(test)]
mod tests;
mod view;

#[cfg(test)]
pub(crate) use sections::help;
