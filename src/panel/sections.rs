use super::{
  App, Entry, PanelKind, PathBuf, Result, Row, Target, help, terminal,
};

impl App {
  /// Follows section changes without interrupting panel browsing.
  pub fn track_outline(&mut self) {
    let mut reading = self.reading_parts();
    reading.outline.track(&mut reading.view, reading.panel);
  }

  pub fn panel_rows(&mut self, kind: PanelKind) -> Result<Vec<Row>> {
    let label = |target: &Target| match target {
      Target::Page(page) | Target::Point { page, .. } => {
        self.doc.get_label(*page).map_or_else(String::new, str::to_owned)
      }
      Target::Uri(_) => String::new(),
    };
    Ok(match kind {
      PanelKind::Outline => self
        .doc
        .outline
        .iter()
        .map(|item| Row::outline(item, label(&item.target)))
        .collect(),
      PanelKind::Results => self
        .search
        .hits()
        .iter()
        .map(|hit| Row::result(hit, self.doc.label(hit.first.page).to_owned()))
        .collect(),
      PanelKind::Links => self.link_rows()?,
      PanelKind::Jumps => self.jump_rows(),
      PanelKind::Annotations => self.annotation_rows()?,
      PanelKind::Help => help(crate::cmdline::on_path)
        .iter()
        .map(|line| Row::parse(line))
        .collect(),
      PanelKind::Info => self.info().lines().map(Row::parse).collect(),
      PanelKind::Text | PanelKind::Recent | PanelKind::Marks => Vec::new(),
    })
  }

  /// PDF metadata, document facts, reader statistics, terminal caps.
  pub fn info(&self) -> String {
    let metadata = self.doc.metadata();
    let bytes = std::fs::metadata(&self.path).map_or(0, |m| m.len());
    let page = self.navigation_position().page;
    vpdf_ui::panel::Info {
      document: vpdf_ui::panel::DocumentInfo {
        metadata: &metadata,
        path: &self.path,
        bytes,
        pages: self.doc.pages.len(),
        label: self.doc.label(page),
        bounds: self.doc.pages[page.0].bounds,
        headings: self.doc.outline.len(),
        inferred: self.doc.inferred,
        marks: self.marks.len(),
        cache_bytes: self.doc.cache_bytes(),
      },
      layout: &self.layout,
      search: &self.search,
      images: self.image_usage,
      render_ms: self.render_ms,
      screen: self.screen,
      text: self.text_mode,
      mouse: terminal::mouse(),
      caps: vpdf_terminal::caps::get(),
    }
    .text()
  }
}

/// Recent documents that still exist, newest first.
pub fn recent_rows(entries: &[Entry]) -> Vec<Row> {
  let home = std::env::var_os("HOME").map(PathBuf::from);
  entries
    .iter()
    .filter(|entry| entry.path.exists())
    .map(|entry| Row::recent(&entry.path, entry.cursor.page, home.as_deref()))
    .collect()
}
