use crate::presentation;
use anyhow::Result;
use std::hint::black_box;
use vpdf_model::geometry::At;
use vpdf_model::geometry::Screen;
use vpdf_model::text::PageId;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::layout::Layout;
use vpdf_ui::paint::Frame;
use vpdf_ui::region::{Canvas, CaptureId, Region, Snapshot};

#[derive(serde::Deserialize)]
struct Prepared {
  page: usize,
  bounds: [f32; 4],
  screen: [u16; 4],
  origin: (u16, u16),
  at: (u16, u16),
  left: u16,
  scroll: f32,
  output_sha256: [String; 2],
}

pub fn draw() -> Result<()> {
  let prepared: Prepared = super::fixture("region")?;
  let geometry: super::Geometry = super::fixture("geometry")?;
  let [columns, rows, cell_width, cell_height] = prepared.screen;
  let screen = Screen {
    origin: prepared.origin,
    ..crate::screen::pixels(columns, rows, cell_width, cell_height)
  };
  let layout =
    Layout::new(geometry.pages.into_iter().map(super::bounds), screen);
  let mut region = Region::open(Snapshot {
    id: CaptureId::new(1),
    page: PageId(prepared.page),
    bounds: super::bounds(prepared.bounds),
  });
  super::measure("Region::input pair", || {
    for motion in
      [vpdf_reader::motion::Motion::Right, vpdf_reader::motion::Motion::Left]
    {
      black_box(region.input(black_box(vpdf_ui::keys::Action::Motion(motion))));
    }
  })?;
  let canvas = Canvas {
    layout: &layout,
    scroll: prepared.scroll,
    frame: Frame {
      screen,
      at: At { column: prepared.at.0, row: prepared.at.1 },
      left: prepared.left,
      text: false,
    },
  };
  let mut graphics = Graphics::default();
  let mut output = Vec::new();
  let mut draw = |out: &mut Vec<u8>| -> Result<()> {
    out.clear();
    graphics.begin(out)?;
    let snap_to_cells = graphics.crop_quirk();
    presentation::draw(out, Some(&mut graphics), |scene| {
      canvas.scene(Some(&mut region), None, snap_to_cells, scene);
    })?;
    graphics.end(out)?;
    Ok(())
  };
  for hash in prepared.output_sha256 {
    draw(&mut output)?;
    anyhow::ensure!(
      super::hash::digest(&output) == hash,
      "Region fixture output differs"
    );
  }
  super::measure("region::Canvas::draw warm", || {
    draw(&mut output).expect("prepared capture region");
    black_box(&output);
  })
}
