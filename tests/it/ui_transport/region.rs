use crate::presentation::*;
use anyhow::Result;
use vpdf_model::text::{Bounds, PageId};
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::layout::Layout;
use vpdf_ui::paint::Frame;
use vpdf_ui::region::*;
fn snapshot(id: u64) -> Snapshot {
  Snapshot {
    id: CaptureId::new(id),
    page: PageId(0),
    bounds: Bounds { x0: 10.0, y0: 20.0, x1: 60.0, y1: 80.0 },
  }
}

#[test]
fn recorded_regions_keep_hits() -> Result<()> {
  use vpdf_model::geometry::At;
  use vpdf_model::geometry::Screen;
  use vpdf_model::scene::Scene;
  use vpdf_terminal::graphics::Presentation;
  use vpdf_terminal::payload::Encoder;
  let screen = Screen {
    columns: 20,
    rows: 10,
    cell_width: 8,
    cell_height: 16,
    origin: (2, 1),
  };
  let layout =
    Layout::new([Bounds { x0: 0.0, y0: 0.0, x1: 160.0, y1: 160.0 }], screen);
  let canvas = Canvas {
    layout: &layout,
    scroll: 0.0,
    frame: Frame { screen, at: At { column: 5, row: 1 }, left: 3, text: false },
  };
  let sweep =
    Some((PageId(0), Bounds { x0: 12.5, y0: 24.5, x1: 63.2, y1: 81.7 }));
  for crop_quirk in [false, true] {
    let mut recorded = Region::open(snapshot(1));
    let mut scene = Scene::new();
    canvas.scene(Some(&mut recorded), sweep, crop_quirk, &mut scene);
    let area = recorded.cells().unwrap();
    assert_eq!(area.column, 3);
    assert_eq!(recorded.items().at(area.column, area.row), Some(0));
    let profile = Presentation { crop_quirk, ..Presentation::default() };
    let mut graphics = Graphics::new(profile, &Encoder::default());
    let mut replay = Graphics::new(profile, &Encoder::default());
    let mut direct = Region::open(snapshot(1));
    for _ in 0..2 {
      let mut expected = Vec::new();
      graphics.begin(&mut expected)?;
      canvas.draw(Some(&mut direct), sweep, &mut graphics, &mut expected)?;
      graphics.end(&mut expected)?;
      assert_eq!(direct.cells(), Some(area));
      let mut actual = Vec::new();
      replay.begin(&mut actual)?;
      let mut painter = vpdf_terminal::painter::Painter::new(&mut actual)
        .with_graphics(&mut replay);
      painter.paint(&scene);
      painter.finish()?;
      replay.end(&mut actual)?;
      assert_eq!(actual, expected);
    }
  }
  Ok(())
}
