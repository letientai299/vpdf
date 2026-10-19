use anyhow::Result;
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::hint::black_box;
use std::time::Instant;
use vpdf_model::document::{Heading, Target};
use vpdf_model::geometry::{Crop, Screen};
use vpdf_model::text::Position;
use vpdf_reader::reader::{Location, Reader};
use vpdf_search::search::Hit;
use vpdf_terminal::{graphics::Graphics, terminal::At};
use vpdf_ui::find::Search;
use vpdf_ui::layout::Layout;
use vpdf_ui::paint::Frame;
use vpdf_ui::pointer::{Region, Route};
use vpdf_ui::scrollbar::{Bar, Input, Ticks, View};

#[derive(serde::Deserialize)]
struct Prepared {
  headings: Vec<(String, usize, Target)>,
  hits: Vec<(Position, Position, String)>,
  labels: Vec<String>,
}

pub fn draw() -> Result<()> {
  let prepared: Prepared = super::fixture("scrollbar")?;
  let geometry: super::Geometry = super::fixture("geometry")?;
  let screen = Screen {
    columns: 120,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let pages: Vec<_> = geometry.pages.into_iter().map(super::bounds).collect();
  let mut layout = Layout::new(pages.iter().copied(), screen);
  layout.restore(2.0, 0, Crop::default());
  layout.reflow(pages.iter().copied());
  let headings: Vec<_> = prepared
    .headings
    .into_iter()
    .map(|(title, depth, target)| Heading { title, depth, target })
    .collect();
  let mut search = Search::default();
  search.show(
    vpdf_search::query::Query::default(),
    prepared
      .hits
      .into_iter()
      .map(|(first, last, excerpt)| Hit { first, last, excerpt })
      .collect(),
  );
  let view = || View {
    layout: &layout,
    scroll: 0.0,
    available: true,
    hovered: true,
    frame: Frame { screen, left: 0, text: false, at: At::default() },
  };
  let ticks = || Ticks { outline: &headings, search: &search };
  let mut bar = Bar::default();
  let route = Route {
    pane: 1,
    region: Region::Page,
    focus: true,
    clicks: 1,
    pixel: None,
    mouse: MouseEvent {
      kind: MouseEventKind::Down(MouseButton::Left),
      column: 119,
      row: 0,
      modifiers: KeyModifiers::NONE,
    },
  };
  let Input::Press(press) = bar.input(&view(), &route) else {
    anyhow::bail!("Missing scrollbar press");
  };
  press.apply(&mut bar, Location::default(), &mut Reader::default());
  let mut graphics = Graphics::default();
  let mut output = Vec::new();
  let mut draw = |out: &mut Vec<u8>| -> Result<()> {
    out.clear();
    graphics.begin(out)?;
    if let Some(label) =
      bar.draw(view(), ticks(), Instant::now(), &mut graphics, out)?
    {
      let text = &prepared.labels[label.page().0];
      label.draw(text, &mut graphics, out)?;
    }
    graphics.end(out)?;
    Ok(())
  };
  for _ in 0..2 {
    draw(&mut output)?;
    println!("scrollbar output {}", super::hash::digest(&output));
  }
  super::measure("Bar::draw warm", || {
    draw(&mut output).expect("prepared scrollbar");
    black_box(&output);
  })
}
