use crate::presentation::*;
use anyhow::Result;
use std::io::Write;
use std::time::Instant;
use vpdf_model::input::MouseEventKind;
use vpdf_model::scene::{Layer, Op};
use vpdf_model::text::{Bounds, PageId};
use vpdf_reader::reader::{Location, Reader};
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::find::Search;
use vpdf_ui::layout::Layout;
use vpdf_ui::paint::Frame;
use vpdf_ui::pointer::Route;
use vpdf_ui::scrollbar::*;

fn layout() -> Layout {
  let bounds = Bounds { x0: 0.0, y0: 0.0, x1: 960.0, y1: 1000.0 };
  Layout::new(
    [bounds; 3],
    vpdf_ui::layout::Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    },
  )
}

fn view(layout: &Layout, scroll: f32) -> View<'_> {
  View {
    layout,
    scroll,
    available: true,
    hovered: true,
    frame: Frame {
      screen: layout.screen(),
      left: 0,
      text: false,
      at: vpdf_model::geometry::At::default(),
    },
  }
}

fn route(kind: MouseEventKind, row: u16) -> Route {
  Route {
    pane: 1,
    region: vpdf_ui::pointer::Region::Page,
    focus: true,
    clicks: 1,
    pixel: None,
    mouse: vpdf_model::input::MouseEvent {
      kind,
      row,
      column: 119,
      modifiers: vpdf_model::input::KeyModifiers::NONE,
    },
  }
}

#[test]
fn label_requests_follow_successful_bar_output() -> Result<()> {
  use vpdf_model::input::{MouseButton::Left, MouseEventKind::Down};
  struct Closed;
  impl Write for Closed {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
      Err(std::io::Error::other("Closed output"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
      Ok(())
    }
  }
  let layout = layout();
  let mut bar = Bar::default();
  let mut reader = Reader::default();
  let search = Search::default();
  let ticks = || Ticks { outline: &[], search: &search };
  let Input::Press(press) =
    bar.input(&view(&layout, 0.0), &route(Down(Left), 0))
  else {
    panic!("Missing thumb press")
  };
  press.apply(&mut bar, Location::default(), &mut reader);
  let now = Instant::now();
  assert!(
    bar
      .draw(
        view(&layout, 0.0),
        ticks(),
        now,
        &mut Graphics::default(),
        &mut Closed
      )
      .is_err()
  );
  let label = bar
    .draw(
      view(&layout, 0.0),
      ticks(),
      now,
      &mut Graphics::default(),
      &mut Vec::new(),
    )?
    .unwrap();
  assert_eq!(label.page(), PageId(0));
  let mut out = Vec::new();
  label.draw("p. 1 · First section", &mut Graphics::default(), &mut out)?;
  assert!(String::from_utf8_lossy(&out).contains(" p. 1 · First section "));
  Ok(())
}

#[test]
fn recorded_scrub_keeps_track_and_label() -> Result<()> {
  use vpdf_model::scene::{FillArea, Scene};
  let layout = layout();
  let view = view(&layout, 100.0);
  let ticks = Ticks { outline: &[], search: &Search::default() };
  let now = Instant::now();
  let mut bar = Bar::default();
  let Input::Press(press) = bar.input(
    &view,
    &route(MouseEventKind::Down(vpdf_model::input::MouseButton::Left), 5),
  ) else {
    panic!("Expected scrollbar press")
  };
  press.apply(&mut bar, Location::default(), &mut Reader::default());
  let mut scene = Scene::new();
  let label = bar.scene(view, ticks, now, &mut scene).unwrap();
  let page = label.page();
  label.scene("Section", &mut scene);
  let Op::Fill(track) = &scene[0] else { panic!("Expected scrollbar track") };
  assert_eq!(
    track.area(),
    FillArea::Cells { column: 119, columns: 1, rows: 39 }
  );
  assert_eq!(track.layer(), Layer::BelowText);
  let mut graphics = Graphics::default();
  let mut replay = Graphics::default();
  for _ in 0..2 {
    let mut expected = Vec::new();
    graphics.begin(&mut expected)?;
    let label =
      bar.draw(view, ticks, now, &mut graphics, &mut expected)?.unwrap();
    assert_eq!(label.page(), page);
    label.draw("Section", &mut graphics, &mut expected)?;
    graphics.end(&mut expected)?;
    let mut actual = Vec::new();
    replay.begin(&mut actual)?;
    let mut painter = vpdf_terminal::painter::Painter::new(&mut actual)
      .with_graphics(&mut replay);
    painter.paint(&scene);
    painter.finish()?;
    replay.end(&mut actual)?;
    assert_eq!(actual, expected);
  }
  Ok(())
}
