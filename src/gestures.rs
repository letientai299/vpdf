//! Pointer selection: click runs, drags, and Alt-click.

use crate::Result;
use crate::app::App;
use crate::figures::Source;
use crate::objects::Object;
use crate::pointer::Route;
use crate::reader::{Mode, Reader, Selection, SelectionKind};
use crate::text::{Bounds, PageId, Position};

/// What a click run's drag extends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stretch {
  /// Words for `Character`, lines for `Line`.
  pub kind: SelectionKind,
  /// The clicked unit's ends.
  pub first: Position,
  pub last: Position,
}

/// An Alt-press region capture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sweep {
  pub page: PageId,
  /// Page point of the press.
  pub from: (f32, f32),
  /// Page point of the drag, once moved.
  pub to: Option<(f32, f32)>,
}

impl Sweep {
  /// The swept rectangle, once it has area.
  pub fn bounds(self) -> Option<Bounds> {
    let ((x0, y0), (x1, y1)) = (self.from, self.to?);
    let bounds =
      Bounds { x0: x0.min(x1), y0: y0.min(y1), x1: x0.max(x1), y1: y0.max(y1) };
    (bounds.width() >= 1.0 && bounds.height() >= 1.0).then_some(bounds)
  }
}

/// What the held left button does.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Gesture {
  /// Plain drags select glyphs.
  #[default]
  Glyphs,
  /// Click-run drags extend by unit.
  Stretch(Stretch),
  /// Alt-drags sweep a capture.
  Sweep(Sweep),
  /// Cancelled until the release.
  Dropped,
}

impl App {
  /// Alt-press: starts a capture; text extends.
  pub(crate) fn alt_press(&mut self, route: &Route) -> Result<()> {
    self.gesture = Gesture::Glyphs;
    if self.text_mode {
      return self.alt_extend(route);
    }
    let (column, row) = (route.mouse.column, route.mouse.row);
    let Some((page, x, y)) = self.page_point(column, row, route.pixel) else {
      return Ok(());
    };
    // Presses off the shown page extend.
    if !self.shown_page(page).contains(x, y) {
      return self.alt_extend(route);
    }
    self.gesture = Gesture::Sweep(Sweep { page, from: (x, y), to: None });
    Ok(())
  }

  /// Alt-drag: moves the capture's corner.
  pub(crate) fn alt_drag(&mut self, route: &Route) -> Result<()> {
    let Gesture::Sweep(Sweep { page, .. }) = self.gesture else {
      return Ok(());
    };
    self.edge_follow(route)?;
    let (column, row) = (route.mouse.column, route.mouse.row);
    let Some((x, y)) = self.point_on(page, column, row, route.pixel) else {
      return Ok(());
    };
    let shown = self.shown_page(page);
    let to = (x.clamp(shown.x0, shown.x1), y.clamp(shown.y0, shown.y1));
    if let Gesture::Sweep(sweep) = &mut self.gesture {
      sweep.to = Some(to);
    }
    Ok(())
  }

  /// Alt-release: sweep, figure, else extend.
  pub(crate) fn alt_release(&mut self) -> Result<()> {
    let Gesture::Sweep(sweep) = std::mem::take(&mut self.gesture) else {
      return Ok(());
    };
    if let Some(bounds) = sweep.bounds() {
      return self.select_region(sweep.page, bounds);
    }
    let (x, y) = sweep.from;
    let figure = self.figure_at(sweep.page, x, y)?;
    // Captions alone do not pick.
    if let Some(figure) =
      figure.filter(|figure| figure.source == Source::Graphic)
    {
      return self.select_region(sweep.page, figure.bounds);
    }
    // Extend where the figure check looked.
    let pos = self.glyph_near(sweep.page, x, y)?;
    self.extend_select(pos)
  }

  /// Extends to the press's glyph.
  fn alt_extend(&mut self, route: &Route) -> Result<()> {
    let Some(pos) = self.press_position(route)? else { return Ok(()) };
    self.extend_select(pos)
  }

  /// Drops a held sweep; whether one was.
  pub(crate) fn drop_sweep(&mut self) -> bool {
    let held = matches!(self.gesture, Gesture::Sweep(_));
    if held {
      self.gesture = Gesture::Dropped;
    }
    held
  }

  /// The page's shown bounds, after crop.
  fn shown_page(&self, page: PageId) -> Bounds {
    self.layout.crop.apply(self.doc.pages[page.0].bounds)
  }

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
    self.gesture = Gesture::Glyphs;
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
    self.gesture = Gesture::Stretch(Stretch { kind, first, last });
    Ok(())
  }

  /// Drags to `pos` by the run's unit.
  pub(crate) fn drag_select(&mut self, pos: Position) -> Result<()> {
    self.reader.desired = None;
    let Gesture::Stretch(Stretch { kind, first, last }) = self.gesture else {
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
    self.gesture = Gesture::Glyphs;
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

  /// Scrolls when a drag nears an edge.
  pub(crate) fn edge_follow(&mut self, route: &Route) -> Result<()> {
    let rows = self.edge_rows(route);
    if rows == 0 {
      return Ok(());
    }
    self.edge_scroll(rows)
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
