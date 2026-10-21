use crate::presentation::ContextDraw;
use anyhow::Result;
use std::hint::black_box;
use vpdf_model::geometry::At;
use vpdf_model::geometry::Screen;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::context::{ContextMenu, Item};
use vpdf_ui::paint::Frame;

#[derive(serde::Deserialize)]
struct Prepared {
  entries: Vec<(String, String)>,
  press: (u16, u16),
  columns: (u16, u16),
  bottom: u16,
  area: [u16; 4],
  screen: [u16; 4],
  origin: (u16, u16),
  at: (u16, u16),
  text: bool,
  row: bool,
  output_sha256: String,
}

pub fn draw() -> Result<()> {
  let prepared: Prepared = super::fixture("context")?;
  let items = prepared
    .entries
    .into_iter()
    .map(|(label, key)| Item { label: label.into(), key, command: () })
    .collect();
  let mut menu = ContextMenu::new(
    items,
    prepared.press,
    prepared.columns,
    prepared.bottom,
    prepared.row,
  );
  super::measure("ContextMenu::input pair", || {
    for motion in
      [vpdf_reader::motion::Motion::Down, vpdf_reader::motion::Motion::Up]
    {
      black_box(menu.input(black_box(vpdf_ui::keys::Action::Motion(motion))));
    }
  })?;
  let area = menu.area();
  anyhow::ensure!(
    [area.column, area.row, area.columns, area.rows] == prepared.area,
    "Menu fixture geometry differs"
  );
  let [columns, rows, cell_width, cell_height] = prepared.screen;
  let frame = Frame {
    screen: Screen {
      columns,
      rows,
      cell_width,
      cell_height,
      origin: prepared.origin,
    },
    at: At { column: prepared.at.0, row: prepared.at.1 },
    left: prepared.columns.0,
    text: prepared.text,
  };
  let mut graphics = Graphics::default();
  let mut out = Vec::new();
  graphics.begin(&mut out)?;
  menu.draw(&mut graphics, &mut out, frame)?;
  graphics.end(&mut out)?;
  anyhow::ensure!(
    super::hash::digest(&out) == prepared.output_sha256,
    "Menu fixture output differs"
  );
  super::measure("ContextMenu::draw", || {
    out.clear();
    graphics.begin(&mut out).expect("begin menu frame");
    menu.draw(&mut graphics, &mut out, frame).expect("prepared menu rows");
    graphics.end(&mut out).expect("menu placements");
    black_box(&out);
  })
}
