//! Pointer gestures: click runs, drags, and buttons.

use crate::Result;
use crate::app::App;
use crate::pdf_text::PdfText;
use crate::text::{Bounds, PageId, Position};
use crossterm::event::MouseEventKind;
use std::time::Instant;
use vpdf_reader::reader::{Mode, SelectionKind};
use vpdf_ui::input::gesture::Swept;
use vpdf_ui::page::landing::Span;
use vpdf_ui::pointer::Route;

impl App {
  /// Alt-press: starts a capture; text extends.
  pub(crate) fn alt_press(&mut self, route: &Route) -> Result<()> {
    self.gesture.reset();
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
    self.gesture.begin_sweep(page, (x, y));
    Ok(())
  }

  /// Alt-drag: moves the capture's corner.
  pub(crate) fn alt_drag(&mut self, route: &Route) -> Result<()> {
    let Some(sweep) = self.gesture.sweep() else { return Ok(()) };
    let page = sweep.page();
    self.edge_follow(route)?;
    self.sweep_to(page, route);
    Ok(())
  }

  /// Moves the capture's corner to the pointer.
  fn sweep_to(&mut self, page: PageId, route: &Route) {
    let (column, row) = (route.mouse.column, route.mouse.row);
    let Some((x, y)) = self.point_on(page, column, row, route.pixel) else {
      return;
    };
    let shown = self.shown_page(page);
    self.gesture.sweep_to((x, y), shown);
  }

  /// Alt-release: sweep, figure, else extend.
  pub(crate) fn alt_release(&mut self) -> Result<()> {
    let Some(sweep) = self.gesture.finish_sweep() else { return Ok(()) };
    match sweep.resolve(&mut PdfText(&mut self.doc))? {
      Swept::Region(page, bounds) => self.select_region(page, bounds),
      Swept::Extend(position) => self.extend_select(position),
    }
  }

  /// Extends to the press's glyph.
  fn alt_extend(&mut self, route: &Route) -> Result<()> {
    let Some(pos) = self.press_position(route)? else { return Ok(()) };
    self.extend_select(pos)
  }

  /// Whether an Alt-press sweeps a capture.
  pub(crate) fn sweeping(&self) -> bool {
    self.gesture.sweep().is_some()
  }

  /// The status hint while a sweep would capture.
  pub(crate) fn sweep_hint(&self) -> Option<String> {
    self.gesture.hint().map(str::to_owned)
  }

  /// Ends held gestures; whether a sweep was.
  pub(crate) fn end_gestures(&mut self) -> bool {
    self.autoscroll.cancel();
    self.drop_sweep()
  }

  /// Drops a held sweep; whether one was.
  pub(crate) fn drop_sweep(&mut self) -> bool {
    self.gesture.drop_sweep()
  }

  /// Middle clicks split links; drags pan.
  pub(crate) fn page_middle(&mut self, route: &Route) -> Result<()> {
    let (column, row) = (route.mouse.column, route.mouse.row);
    let at = self.cell_point(column, row, route.pixel);
    match route.mouse.kind {
      MouseEventKind::Down(_) => {
        self.animation.cancel();
        self.grab.begin(at);
      }
      MouseEventKind::Drag(_) => {
        if self.text_mode {
          return self.grab_text(at);
        }
        let Some((pan, delta)) = self.grab.pixels(at) else { return Ok(()) };
        self.animation.cancel();
        let scroll = self.reader.scroll - delta;
        self.reader.scroll = self.layout.clamp_scroll(scroll);
        self.pan(pan)?;
      }
      _ => {
        if !self.grab.release() {
          return Ok(());
        }
        if let Some(target) = self.link_pressed(column, row, route.pixel)? {
          self.open_split(&target)?;
        }
      }
    }
    Ok(())
  }

  /// Moves text by the whole cells a middle drag crossed.
  fn grab_text(&mut self, at: (f32, f32)) -> Result<()> {
    let cell = (self.screen.cell_width, self.screen.cell_height);
    let Some((columns, rows)) = self.grab.cells(at, cell) else {
      return Ok(());
    };
    self.text().scroll(rows)?;
    self.text().pan(columns)
  }

  /// Whether a middle drag pans.
  pub fn grabbing(&self) -> bool {
    self.grab.moved()
  }

  /// Ends held drags, pans, and scrubs whose release may never come;
  /// whether any was held.
  pub(crate) fn drop_held(&mut self) -> bool {
    self.end_gestures() | self.drop_grab() | self.drop_scrub()
  }

  /// Ends a lost press; whether it panned.
  pub(crate) fn drop_grab(&mut self) -> bool {
    self.grab.cancel()
  }

  /// The page's shown bounds, after crop.
  fn shown_page(&self, page: PageId) -> Bounds {
    self.layout.crop_for(page).apply(self.doc.pages[page.0].bounds)
  }

  /// Click runs: target, word, line, paragraph.
  pub(crate) fn click_select(
    &mut self,
    pos: Position,
    clicks: u8,
  ) -> Result<()> {
    self.target = Some(pos);
    let flash = self.gesture.click(
      &mut self.reader,
      pos,
      clicks,
      &mut PdfText(&mut self.doc),
    )?;
    if flash {
      self.landing.show(pos, Span::Glyph, Instant::now());
    }
    Ok(())
  }

  pub(crate) fn drag_select(&mut self, pos: Position) -> Result<()> {
    self.gesture.drag(&mut self.reader, pos, &mut PdfText(&mut self.doc))
  }

  /// Alt-click: move the end, else start.
  pub(crate) fn extend_select(&mut self, pos: Position) -> Result<()> {
    self.gesture.reset();
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
    let frame = vpdf_ui::paint::Frame {
      screen: self.screen,
      at: self.at,
      left: self.left,
      text: self.text_mode,
    };
    vpdf_ui::input::edge::rows(frame, self.menu_rows(), route)
  }

  pub(crate) fn edge_follow(&mut self, route: &Route) -> Result<()> {
    let rows = self.edge_rows(route);
    if let Some(start) = self.autoscroll.follow(*route, rows) {
      self.edge_scroll(start.rows())?;
      start.begin(&mut self.autoscroll, Instant::now());
    }
    Ok(())
  }

  /// Steps a held edge drag; whether moved.
  pub(crate) fn autoscroll_step(&mut self, now: Instant) -> Result<bool> {
    let Some(held) = self.autoscroll.step(now) else { return Ok(false) };
    // A wheel's easing would hide the step.
    self.animation.cancel();
    let before = self.wheel_view();
    self.edge_scroll(held.rows)?;
    if self.wheel_view() == before {
      // At an end; the next report retries.
      self.autoscroll.cancel();
      return Ok(false);
    }
    if let Some(sweep) = self.gesture.sweep() {
      self.sweep_to(sweep.page(), &held.route);
    } else if !self.gesture.dropped() {
      self.drag_to(&held.route)?;
    }
    Ok(true)
  }

  /// Extends the drag's selection to the pointer.
  pub(crate) fn drag_to(&mut self, route: &Route) -> Result<()> {
    let Some(pos) = self.press_position(route)? else { return Ok(()) };
    self.drag_select(pos)
  }

  /// Scrolls `rows` at once, for drags.
  pub(crate) fn edge_scroll(&mut self, rows: isize) -> Result<()> {
    if self.text_mode {
      self.text().scroll(rows)?;
      // The shown frame is stale.
      self.text_view.invalidate();
      return Ok(());
    }
    let delta = rows as f32 * f32::from(self.screen.cell_height);
    self.reader.scroll = self.layout.clamp_scroll(self.reader.scroll + delta);
    Ok(())
  }
}
