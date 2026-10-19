use anyhow::{Context, Result};
use std::hint::black_box;
use vpdf_model::text::{PageId, Position};
use vpdf_reader::reader::Reader;
use vpdf_ui::input::gesture::Gesture;

pub fn selection() -> Result<()> {
  let prepared = super::fixtures::text()?;
  let line = prepared.text[0]
    .lines
    .iter()
    .position(|line| line.glyphs.len() >= 12)
    .context("No selectable line")?;
  let first = Position { page: PageId(0), line, glyph: 4 };
  let last = Position { glyph: 11, ..first };
  for (name, clicks) in [("word", 2), ("paragraph", 4)] {
    let mut gesture = Gesture::default();
    let mut reader = Reader::default();
    super::measure(&format!("Gesture::click and drag {name}"), || {
      gesture
        .click(&mut reader, black_box(first), clicks, &mut &prepared)
        .expect("prepared selection");
      gesture
        .drag(&mut reader, black_box(last), &mut &prepared)
        .expect("prepared drag");
      black_box((reader.cursor, reader.selection));
    })?;
  }
  Ok(())
}
