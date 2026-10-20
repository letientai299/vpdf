//! Draws a pane: page tiles, text view, and overlays.

use crate::Result;
use crate::app::App;
use crate::pdf_text::PdfText;
use std::io::Write;
use vpdf_model::scene::{Draw, Layer};
use vpdf_model::style::Role;
use vpdf_model::text::{Bounds, PageId, Position};
use vpdf_terminal::graphics::Graphics;
use vpdf_terminal::style::sgr;
use vpdf_terminal::terminal;
use vpdf_ui::layout::Screen;
use vpdf_ui::panel::Pointing;
use vpdf_ui::pointer::{Hit, List};
use vpdf_ui::text_view::{Frame, Pane};

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
      let mut scene =
        vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
      scene.stretch(
        vpdf_ui::theme::PANE_VEIL,
        Layer::AboveText,
        (0, screen.columns, screen.rows.saturating_sub(1)),
        screen,
        left,
      );
      scene.finish()?;
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
    self.overlays.quiet()
      && !self.text_mode
      && self.panel.is_none()
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
    let dim = self.panel_focused();
    let paint = self.reader.marks(&self.search, self.cursorline, dim, &hovered);
    let canvas = vpdf_ui::page::Canvas {
      layout: &self.layout,
      screen,
      left: self.left,
      menu: self.context_rect(),
    };
    canvas.draw(&mut PdfText(&mut self.doc), &paint, graphics, out)
  }

  /// Glyph rectangle in page-area pixels.
  pub fn position_rect(&mut self, cursor: Position) -> Result<Option<Bounds>> {
    self.reading_view().position_rect(cursor)
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
    write!(out, "{}{}", terminal::frame_start(), sgr(Role::Reset))?;
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
    if self.overlays.overview().is_some() {
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
    if let Some(prompt) = self.overlays.prompt() {
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
    if let Some(clues) = self.overlays.clues().shown() {
      clues.draw(
        out,
        (self.at, row - 1),
        self.screen,
        self.pointed_item(List::Clues),
      )?;
    }
    self.status_line(out, row, detail.as_deref())?;
    out.write_all(terminal::HIDE_CURSOR.as_bytes())?;
    Ok(())
  }
}

impl App {
  fn text_frame(&self) -> Frame {
    Frame { screen: self.screen, at: self.at, left: self.left }
  }

  pub fn text(&mut self) -> Pane<'_, PdfText<'_>> {
    let frame = self.text_frame();
    self.text_view.bind(PdfText(&mut self.doc), frame)
  }

  pub fn text_canvas(&mut self, out: &mut impl Write) -> Result<()> {
    let frame = self.text_frame();
    let hovered = self.hover_segments();
    let dim = self.panel_focused();
    let paint = self.reader.marks(&self.search, self.cursorline, dim, &hovered);
    self.text_view.bind(PdfText(&mut self.doc), frame).draw(out, &paint)
  }
}
