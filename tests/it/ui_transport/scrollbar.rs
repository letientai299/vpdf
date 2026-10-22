#[path = "../../support/scrollbar.rs"]
mod fixture;
use crate::presentation::*;
use anyhow::Result;
use fixture::{layout, route, view};
use std::io::Write;
use std::time::Instant;
use vpdf_model::text::PageId;
use vpdf_reader::reader::{Location, Reader};
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::find::Search;
use vpdf_ui::scrollbar::*;

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
    draw(&mut Closed, Some(&mut Graphics::default()), |scene| bar.scene(
      view(&layout, 0.0),
      ticks(),
      now,
      scene
    ))
    .is_err()
  );
  let label = draw(&mut Vec::new(), Some(&mut Graphics::default()), |scene| {
    bar.scene(view(&layout, 0.0), ticks(), now, scene)
  })?
  .unwrap();
  assert_eq!(label.page(), PageId(0));
  let mut out = Vec::new();
  draw(&mut out, Some(&mut Graphics::default()), |scene| {
    label.scene("p. 1 · First section", scene);
  })?;
  assert!(String::from_utf8_lossy(&out).contains(" p. 1 · First section "));
  Ok(())
}

#[test]
fn recorded_scrub_keeps_track_and_label() -> Result<()> {
  let layout = layout();
  let view = view(&layout, 100.0);
  let ticks = Ticks { outline: &[], search: &Search::default() };
  let now = Instant::now();
  let (mut bar, page, scene) = fixture::scrub(&layout, now);
  let mut graphics = Graphics::default();
  let mut replay = Graphics::default();
  for _ in 0..2 {
    let mut expected = Vec::new();
    graphics.begin(&mut expected)?;
    let label = draw(&mut expected, Some(&mut graphics), |scene| {
      bar.scene(view, ticks, now, scene)
    })?
    .unwrap();
    assert_eq!(label.page(), page);
    draw(&mut expected, Some(&mut graphics), |scene| {
      label.scene("Section", scene);
    })?;
    graphics.end(&mut expected)?;
    assert_eq!(replay_frame(&mut replay, &scene)?, expected);
  }
  Ok(())
}
