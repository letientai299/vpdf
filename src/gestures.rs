//! Pointer selection: click runs, drags, and Alt-click.

use crate::Result;
use crate::app::App;
use crate::objects::Object;
use crate::pointer::Route;
use crate::reader::{Mode, Reader, Selection, SelectionKind};
use crate::text::Position;

/// What a click run's drag extends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stretch {
  /// Words for `Character`, lines for `Line`.
  pub kind: SelectionKind,
  /// The clicked unit's ends.
  pub first: Position,
  pub last: Position,
}

impl App {
  /// Click runs: target, word, line, paragraph.
  pub(crate) fn click_select(
    &mut self,
    pos: Position,
    clicks: u8,
  ) -> Result<()> {
    self.reader.end_selection();
    self.reader.cursor = pos;
    self.reader.desired = None;
    self.target = Some(pos);
    self.stretch = None;
    let (kind, object) = match clicks {
      0 | 1 => return Ok(()),
      2 => (SelectionKind::Character, Some(Object::Word)),
      3 => (SelectionKind::Line, None),
      _ => (SelectionKind::Line, Some(Object::Paragraph)),
    };
    let (first, last) = match object {
      Some(object) => self.span_at(pos, object)?.unwrap_or((pos, pos)),
      None => (pos, pos),
    };
    self.reader.selection = Some(Selection { anchor: first, kind });
    self.reader.cursor = last;
    self.stretch = Some(Stretch { kind, first, last });
    Ok(())
  }

  /// Drags to `pos` by the run's unit.
  pub(crate) fn drag_select(&mut self, pos: Position) -> Result<()> {
    self.reader.desired = None;
    let Some(Stretch { kind, first, last }) = self.stretch else {
      if self.reader.selection.is_none() {
        self.reader.visual(SelectionKind::Character);
      }
      self.reader.cursor = pos;
      return Ok(());
    };
    let (start, end) = match kind {
      SelectionKind::Character => {
        self.span_at(pos, Object::Word)?.unwrap_or((pos, pos))
      }
      SelectionKind::Line => (pos, pos),
    };
    // The clicked unit stays whole.
    let (anchor, cursor) =
      if start < first { (last, start) } else { (first, end.max(last)) };
    self.reader.selection = Some(Selection { anchor, kind });
    self.reader.cursor = cursor;
    Ok(())
  }

  /// Alt-click: move the end, else start.
  pub(crate) fn extend_select(&mut self, pos: Position) -> Result<()> {
    self.stretch = None;
    if self.reader.selection.is_none() {
      // Visible Cursor glyphs anchor; else `v`'s.
      let cursor = self.reader.mode() == Mode::Cursor
        && self.visible_position(self.reader.cursor)?;
      if !cursor && !self.enter_text()? {
        return Ok(());
      }
      self.reader.visual(SelectionKind::Character);
    }
    self.reader.cursor = pos;
    self.reader.desired = None;
    Ok(())
  }

  /// Rows an edge drag scrolls.
  pub(crate) fn edge_rows(&self, route: &Route) -> isize {
    let last = self.screen.rows.saturating_sub(2 + self.menu_rows());
    let row = route.mouse.row;
    let Some(pixel) = route.pixel else {
      // Cell reports: the edge rows scroll.
      return match row {
        _ if last == 0 => 0,
        0 => -1,
        _ if row >= last => 1,
        _ => 0,
      };
    };
    let height = i32::from(pixel.cell.1.max(1));
    let top = i32::from(self.at.row) * height;
    let bottom = (i32::from(self.at.row) + i32::from(last) + 1) * height;
    let y = i32::from(pixel.y);
    // Negative reports decode as 0.
    let (direction, past) = if y < top || (y == 0 && top == 0 && row == 0) {
      (-1, (top - y).max(1))
    } else if y >= bottom {
      (1, y - bottom + 1)
    } else {
      return 0;
    };
    direction * (1 + (past - 1) / height) as isize
  }

  /// Scrolls `rows` at once, for drags.
  pub(crate) fn edge_scroll(&mut self, rows: isize) -> Result<()> {
    if self.text_mode {
      self.scroll_text(rows)?;
      // The shown frame is stale.
      self.text_drawn = None;
      return Ok(());
    }
    let delta = rows as f32 * f32::from(self.screen.cell_height);
    self.reader.scroll = self.layout.clamp_scroll(self.reader.scroll + delta);
    Ok(())
  }

  /// An object's ends at `pos`.
  fn span_at(
    &mut self,
    pos: Position,
    object: Object,
  ) -> Result<Option<(Position, Position)>> {
    Reader::object_span(pos, object, false, 1, &mut self.doc)
  }
}
