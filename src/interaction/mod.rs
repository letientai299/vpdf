use crate::Result;
use crate::app::{App, InputOwner};
use crate::gestures::Gesture;
use crate::keys::{Action, Input};
use crate::layout::{Screen, screen_step};
use crate::links::LinkPress;
use crate::panel::Panel;
use crate::pointer::{
  Hit, Items, List, Pixel, Region, Route, WHEEL_ROWS, Wheel,
};
use crate::preview::Origin;
use crate::region::BAR_HINT;
use crate::text::{Bounds, PageId, Position};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use vpdf_reader::motion::Motion;
use vpdf_reader::reader::Location;

/// Zoom steps per wheel turn: touchpads send a turn per cell of travel.
const WHEEL_ZOOM: f32 = 0.5;

/// What the turns of one wheel gesture move.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WheelRun {
  pub surface: Surface,
  /// The first turn's axis; turns across it are drift.
  pub sideways: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Surface {
  Page,
  /// The `K` preview.
  Preview,
}

/// Finds a frame's glyph at a cell.
type GlyphAt = fn(&mut App, u16, u16) -> Result<Option<Position>>;

/// What a wheel turn moves; an equal view needs no frame.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct WheelView {
  scroll: f32,
  pan: f32,
  zoom: f32,
  text: Position,
  /// List and card tops.
  panel: Option<(usize, usize)>,
  overview: Option<usize>,
  clues: Option<usize>,
  preview: Option<i32>,
}

mod hit;
mod mouse;
mod navigation;
mod ownership;
mod view;
mod wheel;
