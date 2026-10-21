use super::*;
#[test]
fn touching_ticks_of_one_color_merge() {
  let (a, b) = ([1; 4], [2; 4]);
  // (ticks) -> runs
  let cases = [
    (vec![(0.0, a), (3.0, a)], vec![(0.0, 5.0, a)]),
    (vec![(0.0, a), (6.0, a)], vec![(0.0, 2.0, a), (6.0, 8.0, a)]),
    (vec![(0.0, a), (3.0, b)], vec![(0.0, 2.0, a), (3.0, 5.0, b)]),
    (vec![], vec![]),
  ];
  for (ticks, want) in cases {
    assert_eq!(runs(ticks.clone()), want, "{ticks:?}");
  }
}

fn layout() -> Layout {
  let bounds = Bounds { x0: 0.0, y0: 0.0, x1: 960.0, y1: 1000.0 };
  Layout::new(
    [bounds; 3],
    crate::layout::Screen {
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
    region: crate::pointer::Region::Page,
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
fn reflow_keeps_user_motion_deadlines() {
  let mut bar = Bar::default();
  let now = Instant::now();
  bar.observe(0.0, now);
  assert_eq!(bar.deadline(), None);
  bar.reflow(0.0, 100.0);
  bar.observe(100.0, now);
  assert_eq!(bar.deadline(), None);
  bar.observe(110.0, now);
  let due = now + Bar::SHOWN;
  assert_eq!(bar.deadline(), Some(due));
  assert!(!bar.expire(due.checked_sub(Duration::from_nanos(1)).unwrap()));
  assert!(bar.expire(due));
  assert!(!bar.expire(due));
  bar.reflow(100.0, 200.0);
  bar.observe(200.0, due);
  assert_eq!(bar.deadline(), Some(due + Bar::SHOWN));
}

#[test]
fn presses_record_only_at_their_boundary() {
  use vpdf_model::input::{
    MouseButton::Left,
    MouseEventKind::{Down, Drag},
  };
  let layout = layout();
  let mut bar = Bar::default();
  let mut reader = Reader::default();
  let origin = Location::default();
  let Input::Press(press) =
    bar.input(&view(&layout, 0.0), &route(Down(Left), 0))
  else {
    panic!("Missing thumb press")
  };
  assert!(!bar.scrubbing());
  assert_eq!(press.apply(&mut bar, origin, &mut reader), None);
  assert_eq!(reader.jumps().count(), 0);
  assert!(bar.release(&mut reader));
  assert_eq!(reader.jumps().count(), 0);
  let Input::Press(press) =
    bar.input(&view(&layout, 0.0), &route(Down(Left), 0))
  else {
    panic!("Missing thumb press")
  };
  press.apply(&mut bar, origin, &mut reader);
  let Input::Scroll(scroll) =
    bar.input(&view(&layout, 0.0), &route(Drag(Left), 5))
  else {
    panic!("Missing thumb drag")
  };
  reader.scroll = scroll;
  assert!(reader.scroll > 0.0);
  assert_eq!(reader.jumps().count(), 0);
  assert!(bar.release(&mut reader));
  assert_eq!(reader.jumps().count(), 1);
  assert!(!bar.release(&mut reader));
  let mut reader = Reader::default();
  let Input::Press(press) =
    bar.input(&view(&layout, 0.0), &route(Down(Left), 20))
  else {
    panic!("Missing track press")
  };
  assert!(!bar.scrubbing());
  assert!(press.apply(&mut bar, origin, &mut reader).unwrap() > 0.0);
  assert_eq!(reader.jumps().count(), 1);
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
