use crate::Result;
use crate::app::App;
use crate::text::{Bounds, PageId, Position};
use std::io::Write;
use vpdf_terminal::graphics::Graphics;
use vpdf_terminal::terminal;
use vpdf_ui::layout::Screen;
use vpdf_ui::panel::Pointing;
use vpdf_ui::pointer::{Hit, List};

impl App {
  /// Keeps finished tiles; returns whether a wanted one arrived, since
  /// prefetched tiles need no frame.
  pub fn raster_updates(&mut self, graphics: &mut Graphics) -> bool {
    self.settle_images(graphics);
    let (mut changed, mut failed) = (false, false);
    for result in self.worker.results.try_iter() {
      match result {
        Ok(tile) => {
          self.render_ms = tile.render_ms;
          changed |= self.tiles.wants(&tile.key);
          graphics.keep(self.id, tile);
        }
        Err(error) => {
          self.error = Some(error);
          failed = true;
        }
      }
    }
    // A tile that never comes must not hold back deferred work.
    if failed {
      self.resume_outline();
    }
    changed || failed
  }

  pub fn canvas(
    &mut self,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<()> {
    self.request_crop();
    let (screen, left) = (self.uncovered(), self.left);
    let view = vpdf_ui::layout::tiles::View {
      layout: &self.layout,
      scroll: self.reader.scroll,
      target: self.reader.animation().target(),
    };
    let frame = vpdf_ui::layout::tiles::Frame { pane: self.id, screen, left };
    let update = self.tiles.draw(view, frame, graphics, out)?;
    self.complete = update.complete;
    if let Some(request) = update.request {
      let nearby =
        if self.crop_scan.active() { Vec::new() } else { self.nearby_pages() };
      self.worker.service.prepare(request.keys(), request.urgent(), &nearby);
    }
    self.marks(graphics, screen, out)?;
    // The side without focus recedes; a veil would dim a menu over it.
    if self.panel_focused() && self.context_rect().is_none() {
      graphics.veil(vpdf_ui::theme::PANE_VEIL, screen, left, out)?;
    }
    Ok(())
  }

  pub(crate) fn nearby_pages(&self) -> Vec<PageId> {
    self.tiles.nearby(
      vpdf_ui::layout::tiles::View {
        layout: &self.layout,
        scroll: self.reader.scroll,
        target: self.reader.animation().target(),
      },
      vpdf_pdf::jobs::Config::default().prefetch_pages,
    )
  }

  /// Whether the frame writes no text above the status line.
  fn plain(&self) -> bool {
    self.overview.is_none()
      && !self.text_mode
      && self.hints.is_none()
      && self.peek.is_none()
      && self.context_menu.is_none()
      && self.region.is_none()
      && self.panel.is_none()
      && self.clues.shown().is_none()
      && self.prompt.is_none()
      && !self.bar.scrubbing()
  }

  pub fn panel_focused(&self) -> bool {
    self.panel.as_ref().is_some_and(|panel| panel.view.focused())
  }

  /// Selection overlays clipped to `screen`.
  pub fn marks(
    &mut self,
    graphics: &mut Graphics,
    screen: Screen,
    out: &mut impl Write,
  ) -> Result<()> {
    let mut hovered = self.hover_segments();
    if hovered.is_empty()
      && let Some((page, areas)) = self.hover_icon()
    {
      hovered.extend(areas.into_iter().map(|bounds| (page, bounds)));
    }
    let paint = vpdf_ui::paint::Marks {
      reader: &self.reader,
      matches: if self.search.hidden() { &[] } else { self.search.hits() },
      flash: self.reader.landing().glyph(&self.reader),
      landing: self.reader.landing().line(),
      cursorline: self.cursorline,
      dim: self.panel_focused(),
      hovered: &hovered,
    };
    let canvas = vpdf_ui::page::Canvas {
      layout: &self.layout,
      screen,
      left: self.left,
      menu: self.context_rect(),
    };
    canvas.draw(
      &mut crate::pdf_text::PdfText(&mut self.doc),
      &paint,
      graphics,
      out,
    )
  }

  /// Glyph rectangle in page-area pixels.
  pub fn position_rect(&mut self, cursor: Position) -> Result<Option<Bounds>> {
    if self.text_mode {
      let cell = self.text().cell(cursor)?;
      let (width, height) =
        (f32::from(self.screen.cell_width), f32::from(self.screen.cell_height));
      return Ok(cell.map(|(column, row)| {
        let (x, y) =
          (f32::from(column - self.left) * width, f32::from(row) * height);
        Bounds { x0: x, y0: y, x1: x + width, y1: y + height }
      }));
    }
    let text = self.doc.text(cursor.page)?;
    Ok(text.glyph(cursor).map(|glyph| {
      self.layout.project(cursor.page, glyph.bounds, self.reader.scroll)
    }))
  }

  /// Deletes the images of a document this pane has since reloaded.
  fn settle_images(&mut self, graphics: &mut Graphics) {
    if std::mem::take(&mut self.images_stale) {
      graphics.forget(self.id);
    }
  }

  /// One frame with this pane alone on screen.
  pub fn draw_alone(
    &mut self,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<()> {
    write!(out, "{}\x1b[0m", terminal::frame_start())?;
    graphics.begin(out)?;
    self.draw_pane(graphics, out)?;
    graphics.end(out)?;
    write!(out, "{}", terminal::frame_end())?;
    out.flush()?;
    Ok(())
  }

  /// Draws within the pane; the caller starts and ends the frame.
  pub fn draw_pane(
    &mut self,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<()> {
    self.settle_images(graphics);
    self.image_usage = graphics.usage();
    // Views without tiles show whole at once; a covered one shows none.
    self.complete = !self.covered();
    // A shrunken terminal can leave no room below the status line.
    if self.screen.rows < 2 || self.screen.columns == 0 {
      return Ok(());
    }
    // Rows above the status line stay blank between plain frames.
    let plain = self.plain();
    if !(plain && self.blank) {
      for row in 0..self.screen.rows {
        terminal::clear(out, self.at, row, self.screen.columns)?;
      }
    }
    self.blank = plain;
    if let Some(panel) = &mut self.panel {
      panel.view.point(match self.hover.hit() {
        Hit::Row(index) => Pointing::Row(index),
        Hit::Card(_) => Pointing::Card,
        _ => Pointing::Away,
      });
    }
    if self.covered()
      && let Some(panel) = &mut self.panel
    {
      // Panes share one picture id; only the focused pane shows it.
      let picture = (!self.text_mode && self.focused).then_some(&mut *graphics);
      panel.draw(out, self.at, self.left, self.screen, picture)?;
      self.draw_context(graphics, out)?;
      return self.bottom(out);
    }
    if self.overview.is_some() {
      self.overview_canvas(graphics, out)?;
    } else if self.text_mode {
      self.text_canvas(out)?;
    } else {
      self.canvas(graphics, out)?;
      self.draw_bar(graphics, out)?;
    }
    self.draw_hints(graphics, out)?;
    self.draw_preview(graphics, out)?;
    self.draw_region(graphics, out)?;
    self.track_outline();
    if let Some(panel) = &mut self.panel {
      panel.draw(out, self.at, self.left, self.screen, None)?;
    }
    // Over the panel too.
    self.draw_context(graphics, out)?;
    self.bottom(out)
  }

  /// Status line, or the prompt with its menu and cursor.
  fn bottom(&mut self, out: &mut impl Write) -> Result<()> {
    let row = self.screen.rows - 1;
    if let Some(prompt) = &self.prompt {
      let pointed = self.pointed_item(List::Completions);
      let (screen, at) = (self.screen, self.at);
      return prompt.draw(out, screen, at, pointed);
    }
    // After the panel, which finds clipped rows.
    // Hidden details cost nothing.
    let detail = if self.shows_detail() {
      self.sweep_hint().or_else(|| self.hover_detail())
    } else {
      None
    };
    if let Some(clues) = self.clues.shown() {
      clues.draw(
        out,
        (self.at, row - 1),
        self.screen,
        self.pointed_item(List::Clues),
      )?;
    }
    self.status_line(out, row, detail.as_deref())?;
    write!(out, "\x1b[?25l")?;
    Ok(())
  }
}
