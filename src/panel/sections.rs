use super::{
  App, Destination, Entry, PageId, PanelKind, Path, PathBuf, Position, Result,
  Row, Style, Target, terminal,
};
use vpdf_model::document::section;

impl App {
  /// Follows section changes without interrupting panel browsing.
  pub fn track_outline(&mut self) {
    let section = self.section();
    let changed = self.current_section != section;
    self.current_section = section;
    let Some(panel) =
      self.panel.as_mut().filter(|panel| panel.kind == PanelKind::Outline)
    else {
      return;
    };
    panel.follow_pending |= changed;
    if !panel.focused
      && panel.follow_pending
      && let Some(index) = section
    {
      panel.select(index);
      panel.follow_pending = false;
    }
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
        .map(|item| Row {
          style: if item.depth == 0 { Style::Heading } else { Style::Plain },
          ..Row::new(
            format!("{}{}", "  ".repeat(item.depth.min(12)), item.title),
            label(&item.target),
            Destination::Link(item.target.clone()),
          )
        })
        .collect(),
      PanelKind::Results => self
        .search
        .hits()
        .iter()
        .map(|hit| {
          Row::new(
            hit.excerpt.clone(),
            self.doc.label(hit.first.page).to_owned(),
            Destination::Position(hit.first),
          )
        })
        .collect(),
      PanelKind::Links => self.link_rows()?,
      PanelKind::Jumps => self.jump_rows(),
      PanelKind::Annotations => self.annotation_rows()?,
      PanelKind::Help => help().iter().map(|line| Row::parse(line)).collect(),
      PanelKind::Info => self.info().lines().map(Row::parse).collect(),
      PanelKind::Text | PanelKind::Recent | PanelKind::Marks => Vec::new(),
    })
  }

  /// PDF metadata, document facts, reader statistics, terminal caps.
  pub fn info(&self) -> String {
    let mut lines = metadata_lines(&self.doc.metadata());
    lines.push(String::new());
    let place = format!("Path: {}", self.path.display());
    let bytes = std::fs::metadata(&self.path).map_or(0, |m| m.len());
    lines.extend(document_lines(place, bytes, self.doc.pages.len()));
    let page = self.navigation_position().page;
    let bounds = self.doc.pages[page.0].bounds;
    let (width, height) = (bounds.width(), bounds.height());
    let outline = if self.doc.inferred { " (inferred)" } else { "" };
    lines.extend([
      format!(
        "Page {}: {width:.0} x {height:.0} pt, {:.0} x {:.0} mm",
        self.doc.label(page),
        width / 72.0 * 25.4,
        height / 72.0 * 25.4
      ),
      format!("Outline: {} entries{outline}", self.doc.outline.len()),
      format!("Marks: {}", self.marks.len()),
      String::new(),
      "# Reader".into(),
      format!("Zoom: {:.0}%", self.layout.zoom * 100.0),
      format!("Rotation: {}°", self.layout.rotation),
      format!(
        "Margins: {}",
        if self.layout.crop.is_none() { "shown" } else { "cropped" }
      ),
      format!(
        "Search: {} hits for {:?}",
        self.search.hits().len(),
        self.search.query().text
      ),
      format!("Text cache: {} KiB", self.doc.cache_bytes() / 1024),
      format!("Images: {} KiB resident", self.image_usage.bytes / 1024),
      format!("Transferred: {} KiB", self.image_usage.uploaded / 1024),
      format!("Last tile: {:.3} ms", self.render_ms),
      format!(
        "Cell: {}x{} px",
        self.screen.cell_width, self.screen.cell_height
      ),
      format!("Screen: {}x{} cells", self.screen.columns, self.screen.rows),
      format!(
        "Presentation: {}",
        if self.text_mode { "text" } else { "images" }
      ),
      format!(
        "Mouse: {}",
        if terminal::mouse() { "on" } else { "off (:mouse on)" }
      ),
      vpdf_terminal::caps::get()
        .map_or_else(|| "Terminal: not probed".into(), ToString::to_string),
    ]);
    lines.join("\n")
  }

  /// Outline entry containing the cursor.
  pub fn section(&mut self) -> Option<usize> {
    if let Some(index) = self.landed_section() {
      return Some(index);
    }
    let cursor = self.navigation_position();
    if self.reader.viewport_mode() && !self.text_mode {
      let y = self
        .layout
        .unproject(cursor.page, (0.0, 0.0), self.reader.scroll)
        .map_or(0.0, |(_, y)| y);
      return section(&self.doc.outline, cursor.page, y);
    }
    self.glyph_section(cursor)
  }

  /// The section `section` names once a jump shows `page` from its
  /// top.
  pub(crate) fn top_section(&mut self, page: PageId) -> Option<usize> {
    if self.reader.viewport_mode() && !self.text_mode {
      return section(&self.doc.outline, page, 0.0);
    }
    self.glyph_section(Position { page, ..Position::default() })
  }

  /// Outline entry containing a glyph's bottom.
  fn glyph_section(&mut self, position: Position) -> Option<usize> {
    let y = self
      .doc
      .text(position.page)
      .ok()
      .and_then(|text| text.glyph(position).map(|g| g.bounds.y1))
      .unwrap_or(f32::MAX);
    section(&self.doc.outline, position.page, y)
  }

  /// The heading a landing put one row below the view top; the top
  /// itself is still in the section before.
  fn landed_section(&mut self) -> Option<usize> {
    let cursor = self.reader.cursor;
    if self.landed != Some(self.landed_view(cursor)) {
      return None;
    }
    // Followed entries name themselves, even sharing a line.
    if let Some((landed, target)) = &self.aimed
      && self.landed == Some(*landed)
      && let Some(index) =
        self.doc.outline.iter().rposition(|heading| heading.target == *target)
    {
      return Some(index);
    }
    let text = self.doc.text(cursor.page).ok()?;
    // Headings land on the glyph nearest their point, as `follow` does.
    self.doc.outline.iter().rposition(|heading| match heading.target {
      Target::Point { page, x, y } if page == cursor.page => {
        text.nearest(page, x, y).line == cursor.line
      }
      _ => false,
    })
  }
}

pub fn metadata_lines(metadata: &[(String, String)]) -> Vec<String> {
  let fields = metadata.iter().map(|(k, v)| format!("{k}: {v}"));
  std::iter::once("# Metadata".to_owned()).chain(fields).collect()
}

/// `:info`'s file facts, shared with recent document cards; `place`
/// names where the file lives.
pub fn document_lines(place: String, bytes: u64, pages: usize) -> Vec<String> {
  vec![
    "# Document".into(),
    place,
    format!("Size: {:.1} MiB", bytes as f64 / 1_048_576.0),
    format!("Pages: {pages}"),
  ]
}

pub(crate) fn help() -> Vec<String> {
  let keys = include_str!("../help.txt").lines().map(str::to_owned);
  let commands = crate::cmdline::commands().map(|spec| {
    let usage = if spec.usage.is_empty() { "" } else { " " };
    format!(":{}{usage}{}: {}", spec.name, spec.usage, spec.help)
  });
  keys.chain([String::new(), "# Commands".into()]).chain(commands).collect()
}

/// Recent documents that still exist, newest first.
pub fn recent_rows(entries: &[Entry]) -> Vec<Row> {
  let home = std::env::var_os("HOME").map(PathBuf::from);
  entries
    .iter()
    .filter(|entry| entry.path.exists())
    .map(|entry| {
      let path = match home.as_deref().map(|h| entry.path.strip_prefix(h)) {
        Some(Ok(rest)) => Path::new("~").join(rest),
        _ => entry.path.clone(),
      };
      Row {
        style: Style::Path,
        ..Row::new(
          path.display().to_string(),
          format!("p{}", entry.cursor.page.0 + 1),
          Destination::File(entry.path.clone()),
        )
      }
    })
    .collect()
}
