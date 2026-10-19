use anyhow::Result;
use std::hint::black_box;
use vpdf_model::geometry::Screen;
use vpdf_model::text::{PageId, Position};
use vpdf_reader::reader::{Reader, Selection, SelectionKind};
use vpdf_terminal::{graphics::Graphics, terminal::At};
use vpdf_ui::layout::Layout;
use vpdf_ui::page::Canvas;
use vpdf_ui::paint::Marks;
use vpdf_ui::text_view::{Frame, TextView};

#[derive(serde::Deserialize)]
struct Prepared {
  name: String,
  source: String,
  text_mode: bool,
  screen: [u16; 4],
  origin: (u16, u16),
  at: (u16, u16),
  left: u16,
  scroll: f32,
  cursor: Position,
  selection: Option<(Position, bool)>,
  cursor_mode: bool,
  top: Position,
  landing: Option<Position>,
  cursorline: bool,
  hovered: Vec<(PageId, [f32; 4])>,
  output_sha256: [String; 2],
}

pub fn draw(filter: &str) -> Result<()> {
  let cases: Vec<Prepared> = super::fixture("overlays")?;
  for case in cases.into_iter().filter(|c| c.name.contains(filter)) {
    run(case)?;
  }
  Ok(())
}

fn run(case: Prepared) -> Result<()> {
  let prepared = super::fixtures::text_named(&case.source)?;
  let [columns, rows, cell_width, cell_height] = case.screen;
  let screen =
    Screen { columns, rows, cell_width, cell_height, origin: case.origin };
  let layout = Layout::new(prepared.metadata.iter().map(|p| p.1), screen);
  let mut reader = Reader::default();
  reader.scroll = case.scroll;
  reader.cursor = case.cursor;
  if case.cursor_mode {
    reader.enter_cursor();
  }
  reader.selection = case.selection.map(|(anchor, linewise)| Selection {
    anchor,
    kind: if linewise { SelectionKind::Line } else { SelectionKind::Character },
  });
  let hovered: Vec<_> = case
    .hovered
    .into_iter()
    .map(|(page, bounds)| (page, super::bounds(bounds)))
    .collect();
  let marks = Marks {
    reader: &reader,
    matches: &[],
    flash: None,
    landing: case.landing,
    cursorline: case.cursorline,
    dim: false,
    hovered: &hovered,
  };
  let canvas = Canvas { layout: &layout, screen, left: case.left, menu: None };
  let mut state = TextView::default();
  let frame = Frame {
    screen,
    left: case.left,
    at: At { column: case.at.0, row: case.at.1 },
  };
  let mut pane = state.bind(&prepared, frame);
  pane.restore(case.top)?;
  let mut graphics = Graphics::default();
  let mut out = Vec::new();
  let mut draw = |out: &mut Vec<u8>| -> Result<()> {
    out.clear();
    if case.text_mode {
      pane.draw(out, &marks)?;
    } else {
      graphics.begin(out)?;
      canvas.draw(&mut &prepared, &marks, &mut graphics, out)?;
      graphics.end(out)?;
    }
    Ok(())
  };
  for hash in case.output_sha256 {
    draw(&mut out)?;
    anyhow::ensure!(
      super::hash::digest(&out) == hash,
      "Overlay output differs: {} text={}",
      case.name,
      case.text_mode
    );
  }
  super::measure(&format!("{} text={}", case.name, case.text_mode), || {
    draw(&mut out).expect("prepared page overlays");
    black_box(&out);
  })
}
