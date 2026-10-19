use crate::Result;
use crate::app::App;
use crate::gestures::Gesture;
use crate::links::LinkPress;
use crate::preview::Origin;
use crate::region::BAR_HINT;
use crate::text::{Bounds, PageId, Position};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use vpdf_reader::motion::Motion;
use vpdf_reader::reader::Location;
use vpdf_ui::input::InputOwner;
use vpdf_ui::keys::{Action, Input};
use vpdf_ui::layout::{Screen, screen_step};
use vpdf_ui::panel::Panel;
use vpdf_ui::pointer::{
  Hit, Items, List, Pixel, Region, Route, WHEEL_ROWS, Wheel,
};

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

impl App {
  pub fn pointer_pane(&self) -> vpdf_ui::pointer::Pane {
    let rect = vpdf_ui::split::Rect {
      column: self.at.column,
      row: self.at.row,
      columns: self.screen.columns,
      rows: self.screen.rows,
    };
    vpdf_ui::pointer::Pane {
      id: self.id,
      rect,
      panel: self.left,
      overlay: self.menu_rows(),
      overview: self.overview.is_some(),
      preview: self.preview_cells(),
      menu: self.context_menu.as_ref().map(crate::context::ContextMenu::area),
      actions: self.bar_cells(),
    }
  }
}
