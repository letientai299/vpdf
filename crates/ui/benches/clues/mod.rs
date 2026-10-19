use anyhow::Result;
use std::hint::black_box;
use vpdf_model::geometry::Screen;
use vpdf_model::text::{PageId, Position};
use vpdf_terminal::terminal::At;
use vpdf_ui::clues::{self, Clues};

pub fn draw() -> Result<()> {
  let prepared = super::fixtures::text()?;
  anyhow::ensure!(prepared.text.len() <= 26, "Too many benchmark pages");
  let entries: Vec<_> = prepared
    .text
    .iter()
    .enumerate()
    .map(|(page, text)| {
      let name = char::from(b'a' + u8::try_from(page).expect("bounded pages"));
      let line =
        text.lines.first().map_or(String::new(), vpdf_model::text::Line::text);
      (name, line, Position { page: PageId(page), line: 0, glyph: 0 })
    })
    .collect();
  for (columns, rows) in [(80, 24), (30, 10)] {
    let screen =
      Screen { columns, rows, cell_width: 8, cell_height: 16, origin: (0, 0) };
    for prefix in ["g", "\"", "'"] {
      let registers =
        entries.iter().map(|(name, text, _)| (*name, text.as_str()));
      let marks = entries.iter().map(|(name, _, position)| (*name, *position));
      let mut clues = Clues::default();
      assert!(clues.show(clues::items(prefix, registers, marks)));
      let list = clues.shown().expect("prepared clues");
      let mut out = Vec::new();
      let bottom = (At { column: 3, row: 2 }, rows - 2);
      list.draw(&mut out, bottom, screen, None)?;
      println!(
        "clues {prefix:?} {columns}x{rows} output {}",
        super::hash::digest(&out)
      );
      super::measure(
        &format!("Clues::rows {prefix:?} {columns}x{rows}"),
        || {
          black_box(clues.rows(black_box(screen)));
        },
      )?;
      super::measure(
        &format!("Clues::draw {prefix:?} {columns}x{rows}"),
        || {
          out.clear();
          list.draw(&mut out, bottom, screen, None).expect("prepared clues");
          black_box(&out);
        },
      )?;
    }
  }
  Ok(())
}
