use crate::Result;
use crate::app::App;
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use vpdf_model::text::{PageId, Position};
use vpdf_reader::reader::Location;
use vpdf_ui::input::InputOwner;
use vpdf_ui::keys::{Action, Input};
use vpdf_ui::layout::Screen;
use vpdf_ui::panel::Panel;
use vpdf_ui::pointer::{
  Hit, Items, List, Pixel, Region, Route, WHEEL_ROWS, Wheel,
};
use vpdf_ui::preview::Origin;
use vpdf_ui::region::BAR_HINT;

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
      menu: self.context_menu.as_ref().map(vpdf_ui::context::ContextMenu::area),
      actions: self.region.as_ref().and_then(vpdf_ui::region::Region::cells),
    }
  }
}
