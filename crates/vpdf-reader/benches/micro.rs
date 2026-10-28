#[path = "../../vpdf-dev/support/directory.rs"]
mod directory;
use anyhow::Result;
use std::hint::black_box;
use vpdf_model::text::{PageId, Position};
use vpdf_reader::motion::Motion;
use vpdf_reader::reader::Reader;
#[path = "../../vpdf-dev/support/fixtures.rs"]
mod fixtures;
#[path = "../../vpdf-dev/support/hash.rs"]
mod hash;
#[path = "../../vpdf-dev/support/mod.rs"]
mod support;
#[path = "../../vpdf-dev/support/text_source.rs"]
mod text_source;
use support::measure;

fn reflow() -> Result<()> {
  let prepared = fixtures::text()?;
  let lines = prepared.text[1].lines.len();
  for (name, count) in
    [("yy paper p2 whole", lines), ("yy paper p2 3 lines", 3)]
  {
    measure(name, || {
      let mut reader = Reader::default();
      reader.cursor.page = PageId(1);
      reader.yank(1, '"', &mut &prepared).expect("operator");
      reader.yank(count, '"', &mut &prepared).expect("paper lines");
      black_box(reader.pending_yank);
    })?;
  }
  Ok(())
}

fn lexical_objects() -> Result<()> {
  let prepared = fixtures::text()?;
  let text = &prepared.text[0];
  let line = text.lines.iter().position(|line| line.glyphs.len() > 15).unwrap();
  let cursor = Position { page: PageId(0), line, glyph: 4 };
  let mut reader = Reader::default();
  measure("Reader::object paper", || {
    reader.cursor = cursor;
    reader.object(false, &mut &prepared).expect("paper word");
    black_box(reader.selection);
  })?;
  Ok(())
}

fn motion() -> Result<()> {
  let prepared = fixtures::text()?;
  let mut reader = Reader::default();
  reader.cursor = Position { page: PageId(0), line: 10, glyph: 4 };
  anyhow::ensure!(
    prepared.text[0].glyph(reader.cursor).is_some(),
    "Missing benchmark cursor"
  );
  measure("Reader::motion down/up", || {
    for motion in [Motion::Down, Motion::Up] {
      reader.motion(motion, 1, false, &mut &prepared).expect("paper motion");
    }
    black_box(reader.cursor);
  })
}

fn main() -> Result<()> {
  support::run(&[
    ("reflow", reflow),
    ("lexical objects", lexical_objects),
    ("motion", motion),
  ])
}
