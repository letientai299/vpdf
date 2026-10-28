use crate::presentation;
use anyhow::{Context, Result};
use std::hint::black_box;
use vpdf_model::geometry::At;
use vpdf_model::text::{PageId, Position};
use vpdf_reader::reader::Reader;
use vpdf_ui::paint::Marks;
use vpdf_ui::text_view::{Frame, TextView};

pub fn canvas() -> Result<()> {
  let prepared = super::fixtures::text()?;
  let screen = crate::screen::cells(120, 40);
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
    links: vpdf_ui::paint::LinkMarks::default(),
  };
  let mut state = TextView::default();
  let mut pane = state.bind(&prepared, Frame::new(screen, At::default(), 0));
  let mut out = Vec::new();
  presentation::try_draw(&mut out, None, |scene| pane.scene(scene, &paint))?;
  println!("text output {}", super::hash::digest(&out));
  super::measure("TextView::draw", || {
    out.clear();
    presentation::try_draw(&mut out, None, |scene| pane.scene(scene, &paint))
      .expect("prepared page text");
    black_box(&out);
  })
}
