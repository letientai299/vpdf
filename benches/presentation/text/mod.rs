use crate::presentation::TextDraw;
use anyhow::{Context, Result};
use std::hint::black_box;
use vpdf_model::geometry::At;
use vpdf_model::geometry::Screen;
use vpdf_model::text::{PageId, Position};
use vpdf_reader::reader::Reader;
use vpdf_ui::paint::Marks;
use vpdf_ui::text_view::{Frame, TextView};

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
  println!("text output {}", super::hash::digest(&out));
  super::measure("TextView::draw", || {
    out.clear();
    pane.draw(&mut out, &paint).expect("prepared page text");
    black_box(&out);
  })
}
