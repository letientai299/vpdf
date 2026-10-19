use crate::Result;
use crate::app::App;
use crate::document::Document;
use crate::raster::Raster;
use vpdf_ui::keys::PanelKind;

impl App {
  /// Releases what a hidden pane rebuilds on demand: the search worker
  /// with its indexes, cached text and display lists, and the render
  /// coordinator's document with every page it loaded. A running search
  /// resumes when the pane shows again.
  pub fn trim(&mut self) {
    self.crop_scan.clear();
    self.search_worker = None;
    self.doc.trim();
    self.worker = Raster::idle(
      &self.path,
      &self.password,
      self.doc.lists(),
      &self.wake,
      vpdf_terminal::payload::Encoder::live(),
    );
    self.tiles.invalidate();
  }

  /// Reopens the document, keeping the view where it still applies.
  pub fn reload_with(&mut self, password: &str) -> Result<()> {
    // A write during the open shows up as a later change.
    let stamp = crate::watch::stamp(&self.path);
    self.doc = Document::open(&self.path, password)?;
    self.crop_scan.clear();
    self.layout.clear_crops();
    self.layout.reflow(self.doc.pages.iter().map(|page| page.bounds));
    self.stamp = stamp;
    password.clone_into(&mut self.password);
    if !self.layout.crop().is_none() {
      self.start_crop();
    }
    self.worker = Raster::new(
      &self.path,
      password,
      self.doc.lists(),
      &self.wake,
      vpdf_terminal::payload::Encoder::live(),
    );
    self.search_worker = None;
    let rerun = self.search.reset_document();
    // An open outline panel waits on the new outline now.
    self.outline_load = crate::outline::Loading::Deferred;
    if self
      .panel
      .as_ref()
      .is_some_and(|panel| panel.view.kind() == PanelKind::Outline)
    {
      self.load_outline();
    }
    self.images_stale = true;
    if let Some(ocr) = self.ocr.take() {
      ocr.cancel();
    }
    self.reader.cancel();
    self.reader.clear_history();
    self.tiles.invalidate();
    self.reading_view().reclamp_cursor()?;
    self.text().reclamp()?;
    self.text_view.invalidate();
    self.reader.clear_target();
    self.reader.clear_flash();
    self.point(None);
    self.link_press.cancel();
    self.dismiss_overlay();
    let mut reading = self.reading_parts();
    reading.view.reclamp_search(reading.search)?;
    if let Some(panel) = &self.panel {
      let kind = panel.view.kind();
      let rows = self.panel_rows(kind)?;
      // These rows come from the session or refill when work finishes.
      let refills = match kind {
        PanelKind::Recent | PanelKind::Marks => true,
        PanelKind::Results => rerun.is_some(),
        PanelKind::Outline => self.outline_pending(),
        _ => false,
      };
      let note = self.panel_note(kind);
      if let Some(panel) = self.panel.as_mut().filter(|_| !rows.is_empty()) {
        panel.view.refill(rows, note);
      } else if !refills {
        self.panel = None;
      }
    }
    if let Some(query) = rerun {
      self.rerun(&query)?;
    }
    self.relayout()?;
    self.reader.notice("Reloaded");
    Ok(())
  }

  /// Copies another pane's view of the same document, as a new split.
  pub fn mirror(&mut self, other: &Self) -> Result<()> {
    self.crop_scan.clear();
    self.layout.copy_view(&other.layout);
    self.doc.margins.clone_from(&other.doc.margins);
    self.text_mode = other.text_mode;
    self.preview = other.preview;
    self.layout.reflow(self.doc.pages.iter().map(|page| page.bounds));
    let shown = other.navigation_position();
    self.reading_view().mirror(shown, other.reader.scroll, &other.text_view)?;
    if !self.layout.crop().is_none() && self.layout.crops().is_empty() {
      self.start_crop();
    }
    Ok(())
  }
}
