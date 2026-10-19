use super::fixtures::TextPages;
use anyhow::{Context, Result};
use std::hint::black_box;
use std::sync::Arc;
use vpdf_model::geometry::Screen;
use vpdf_model::text::{Bounds, PageId, Position, Text};
use vpdf_reader::reader::{Reader, TextSource};
use vpdf_terminal::terminal::At;
use vpdf_ui::paint::Marks;
use vpdf_ui::source::Pages;
use vpdf_ui::text_view::{Frame, TextView};

impl TextSource for &TextPages {
  fn page_count(&self) -> usize {
    self.text.len()
  }
  fn page_text(&mut self, page: PageId) -> Result<Arc<Text>> {
    Ok(Arc::clone(&self.text[page.0]))
  }
}

impl Pages for &TextPages {
  fn count(&self) -> usize {
    self.text.len()
  }
  fn bounds(&self, page: PageId) -> Bounds {
    self.metadata[page.0].1
  }
  fn label(&self, page: PageId) -> &str {
    &self.metadata[page.0].0
  }
}

pub fn canvas() -> Result<()> {
  let prepared = super::fixtures::text()?;
  let screen = Screen {
    columns: 120,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let (rows, columns) = (usize::from(screen.rows), usize::from(screen.columns));
  let line = prepared.text[0]
    .lines
    .iter()
    .take((rows - 1) / 2)
    .position(|line| line.glyphs.len() >= columns / 2)
    .context("No long line near the top")?;
  let mut reader = Reader::default();
  reader.cursor = Position { page: PageId(0), line, glyph: columns / 2 - 1 };
  let paint = Marks {
    reader: &reader,
    matches: &[],
    flash: None,
    landing: None,
    cursorline: true,
    dim: false,
    hovered: &[],
  };
  let mut state = TextView::default();
  let mut pane =
    state.bind(&prepared, Frame { screen, at: At::default(), left: 0 });
  let mut out = Vec::new();
  pane.draw(&mut out, &paint)?;
  super::measure("TextView::draw", || {
    out.clear();
    pane.draw(&mut out, &paint).expect("prepared page text");
    black_box(&out);
  })
}
