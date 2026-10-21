//! Draws a pane: page tiles, text view, and overlays.

use crate::Result;
use crate::app::App;
use crate::pdf_text::PdfText;
use std::io::Write;
use vpdf_model::scene::{Draw, Layer, Op, Surface};
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
  /// Keeps tiles; whether content or statistics changed.
  pub fn raster_updates(&mut self, graphics: &mut Graphics) -> bool {
    self.settle_images(graphics);
    let info = self
      .panel
      .as_ref()
      .is_some_and(|panel| panel.view.kind() == vpdf_ui::keys::PanelKind::Info);
    let (mut changed, mut failed) = (false, false);
    for result in self.worker.results.try_iter() {
      match result {
        Ok(tile) => {
          self.render_ms = tile.render_ms;
          changed |= self.tiles.wants(&tile.key) || info;
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
    let mut painter =
      vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
    let result = self.scene_canvas(&mut painter);
    painter.finish()?;
    result
  }

  pub(crate) fn scene_canvas<'a, S: Surface<'a>>(
    &mut self,
    scene: &mut S,
  ) -> Result<()>
  where
    S::Error: Into<anyhow::Error>,
  {
    self.request_crop();
    let (screen, left) = (self.uncovered(), self.left);
    let view = vpdf_ui::layout::tiles::View {
      layout: &self.layout,
      scroll: self.reader.scroll,
      target: self.reader.animation().target(),
    };
    let frame = vpdf_ui::layout::tiles::Frame { pane: self.id, screen, left };
    let update = self.tiles.scene(view, frame, scene).map_err(Into::into)?;
    self.complete = update.complete;
    if let Some(request) = update.request {
      let nearby =
        if self.crop_scan.active() { Vec::new() } else { self.nearby_pages() };
      self.worker.service.prepare(request.keys(), request.urgent(), &nearby);
    }
    self.scene_marks(scene, screen)?;
    // The side without focus recedes; a veil would dim a menu over it.
    if self.panel_focused() && self.context_rect().is_none() {
      scene.stretch(
        vpdf_ui::theme::PANE_VEIL,
        Layer::AboveText,
        (0, screen.columns, screen.rows.saturating_sub(1)),
        screen,
        left,
      );
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
    let mut painter =
      vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
    let result = self.scene_marks(&mut painter, screen);
    painter.finish()?;
    result
  }

  fn scene_marks<'a>(
    &mut self,
    scene: &mut impl Draw<'a>,
    screen: Screen,
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
    canvas.scene(&mut PdfText(&mut self.doc), &paint, scene)
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
    let snap_regions = graphics.crop_quirk();
    let mut painter =
      vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
    let result = self.paint_pane(&mut painter, snap_regions);
    painter.finish()?;
    result
  }

  pub(crate) fn paint_pane(
    &mut self,
    painter: &mut vpdf_terminal::painter::Painter<'_, impl Write>,
    snap_regions: bool,
  ) -> Result<()> {
    painter.checkpoint()?;
    let graphics = painter.graphics().expect("attached graphics");
    self.settle_images(graphics);
    self.image_usage = graphics.usage();
    let result = self.scene_pane(painter, snap_regions);
    painter.checkpoint()?;
    result
  }

  pub(crate) fn scene_pane<'a, S: Surface<'a>>(
    &mut self,
    scene: &mut S,
    snap_regions: bool,
  ) -> Result<()>
  where
    S::Error: Into<anyhow::Error>,
  {
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
        scene.extend([
          Op::Move {
            column: usize::from(self.at.column),
            row: usize::from(self.at.row) + usize::from(row),
          },
          Op::Erase(self.screen.columns),
        ]);
      }
    }
    self.blank = plain;
    if let Some(panel) = &mut self.panel {
      panel.view.point(match self.hover.hit() {
        Hit::Row(index) | Hit::Label(index) => Pointing::Row(index),
        Hit::Card(_) => Pointing::Card,
        _ => Pointing::Away,
      });
    }
    if self.covered()
      && let Some(panel) = &mut self.panel
    {
      // Panes share one picture id; only the focused pane shows it.
      let picture = !self.text_mode && self.focused;
      panel
        .scene(scene, self.at, self.left, self.screen, picture)
        .map_err(Into::into)?;
      self.scene_context(scene);
      return self.scene_bottom(scene);
    }
    if self.overlays.overview().is_some() {
      self.scene_overview(scene)?;
    } else if self.text_mode {
      self.scene_text(scene)?;
    } else {
      self.scene_canvas(scene)?;
      self.scene_bar(scene);
    }
    self.scene_hints(scene);
    self.scene_preview(scene)?;
    self.scene_region(scene, snap_regions);
    self.track_outline();
    self.refill_panel(vpdf_ui::keys::PanelKind::Info)?;
    if let Some(panel) = &mut self.panel {
      panel
        .scene(scene, self.at, self.left, self.screen, false)
        .map_err(Into::into)?;
    }
    // Over the panel too.
    self.scene_context(scene);
    self.scene_bottom(scene)
  }

  /// Status line, or the prompt with its menu and cursor.
  fn scene_bottom<'a>(&mut self, scene: &mut impl Draw<'a>) -> Result<()> {
    let row = self.screen.rows - 1;
    if let Some(prompt) = self.overlays.prompt() {
      let pointed = self.pointed_item(List::Completions);
      let (screen, at) = (self.screen, self.at);
      prompt.scene(scene, screen, at, pointed);
      return Ok(());
    }
    // After the panel, which finds clipped rows.
    // Hidden details cost nothing.
    let detail = if self.shows_detail() {
      self.sweep_hint().or_else(|| self.hover_detail())
    } else {
      None
    };
    if let Some(clues) = self.overlays.clues().shown() {
      clues.scene(
        scene,
        (self.at, row - 1),
        self.screen,
        self.pointed_item(List::Clues),
      );
    }
    self.chrome_view(&self.name()).scene(
      scene,
      self.at,
      row,
      self.focused,
      detail.as_deref(),
    )?;
    scene.push(Op::HideCursor);
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
    let mut painter = vpdf_terminal::painter::Painter::new(out);
    let result = self.scene_text(&mut painter);
    painter.finish()?;
    result
  }

  fn scene_text<'a>(&mut self, scene: &mut impl Draw<'a>) -> Result<()> {
    let frame = self.text_frame();
    let hovered = self.hover_segments();
    let dim = self.panel_focused();
    let paint = self.reader.marks(&self.search, self.cursorline, dim, &hovered);
    self.text_view.bind(PdfText(&mut self.doc), frame).scene(scene, &paint)
  }
}
