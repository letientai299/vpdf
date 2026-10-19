use super::{Entry, State, now};
use crate::Result;
use crate::app::App;
use crate::text::{PageId, Position};

impl State {
  /// Records where `app` is; a covered pane records nothing. Returns
  /// whether anything moved.
  pub fn remember_pane(&mut self, app: &App) -> bool {
    app.saved_entry().is_some_and(|entry| self.remember(entry))
  }
}

impl App {
  /// Where this pane is, to save; a covered document counts as closed,
  /// since its pane may be a stale copy.
  pub fn saved_entry(&self) -> Option<Entry> {
    if self.covered() {
      return None;
    }
    Some(Entry {
      path: self.path.clone(),
      cursor: self.navigation_position(),
      zoom: self.layout.zoom,
      rotation: self.layout.rotation,
      used: now(),
      marks: self.marks.clone(),
      crop: self.layout.crop,
    })
  }

  /// Returns to a saved position and view.
  pub fn restore(&mut self, entry: &Entry) -> Result<()> {
    self.crop_job = None;
    self.layout.zoom = entry.zoom.clamp(0.1, 8.0);
    self.marks.clone_from(&entry.marks);
    self.layout.crop = entry.crop;
    self.layout.rotation = entry.rotation % 360 / 90 * 90;
    self.layout.reflow(&self.doc.pages);
    let page = PageId(entry.cursor.page.0.min(self.doc.pages.len() - 1));
    let cursor = Position { page, ..entry.cursor };
    self.reader.cursor = self.doc.text(page)?.clamp(cursor);
    self.show_page(page)?;
    self.target = Some(self.reader.cursor);
    self.ensure_cursor()?;
    if !entry.crop.is_none() && self.layout.crops.is_empty() {
      self.start_crop();
    }
    Ok(())
  }
}
