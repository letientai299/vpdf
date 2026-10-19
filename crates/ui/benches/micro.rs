use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::hint::black_box;
use vpdf_model::geometry::Screen;
use vpdf_model::text::PageId;
use vpdf_ui::layout::{Layout, visible};
mod clues;
mod context;
#[path = "../../../benches/support/fixtures.rs"]
mod fixtures;
mod gestures;
mod overlays;
mod page;
use fixtures::{bounds, read as fixture};
#[path = "../../../benches/support/hash.rs"]
mod hash;
mod jumps;
mod reading;
mod region;
mod scrollbar;
mod sections;
#[path = "../../../benches/support/mod.rs"]
mod support;
mod text;
mod tiles;
use support::measure;

#[derive(serde::Deserialize)]
struct Geometry {
  pages: Vec<[f32; 4]>,
  glyph: [f32; 4],
}

fn projection() -> Result<()> {
  let geometry: Geometry = fixture("geometry")?;
  let pages: Vec<_> = geometry.pages.into_iter().map(bounds).collect();
  let glyph = bounds(geometry.glyph);
  let mut layout = Layout::new(
    pages.iter().copied(),
    Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    },
  );
  for zoom in [0.5, 2.0] {
    layout.request_zoom(vpdf_ui::layout::Zoom::To(zoom));
    layout.pan_to(96.0);
    layout.reflow(pages.iter().copied());
    measure(&format!("Layout::project zoom {zoom}"), || {
      black_box(layout.project(black_box(PageId(0)), black_box(glyph), 27.0));
    })?;
    measure(&format!("tiles::visible zoom {zoom}"), || {
      black_box(visible(black_box(&layout), 27.0));
    })?;
  }
  Ok(())
}

fn key_prefixes() -> Result<()> {
  for (name, input) in
    [("Keys::feed navigation", "12j2gg"), ("Keys::feed sections", "2[[3]]")]
  {
    let events: Vec<_> = input
      .chars()
      .map(|ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
      .collect();
    measure(name, || {
      let mut keys = vpdf_ui::keys::Keys::default();
      for event in &events {
        black_box(
          keys.feed(black_box(*event), vpdf_ui::keys::Grammar::Objects),
        );
      }
    })?;
  }
  Ok(())
}

/// A drag across a two-pane split.
fn route() -> Result<()> {
  use crossterm::event::{
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
  };
  use vpdf_ui::pointer::{Capture, Pane, Region};
  use vpdf_ui::split::Rect;
  let pane = |id, column, panel| Pane {
    id,
    rect: Rect { column, row: 0, columns: 60, rows: 40 },
    panel,
    overlay: 3,
    overview: false,
    preview: None,
    menu: None,
    actions: None,
  };
  let panes = [pane(1, 0, 20), pane(2, 61, 0)];
  let capture =
    Some(Capture { pane: 1, button: MouseButton::Left, region: Region::Page });
  let drag = MouseEvent {
    kind: MouseEventKind::Drag(MouseButton::Left),
    column: 90,
    row: 39,
    modifiers: KeyModifiers::NONE,
  };
  measure("pointer::route", || {
    black_box(vpdf_ui::pointer::route(
      black_box(&panes),
      1,
      black_box(capture),
      black_box(drag),
    ));
  })
}

fn panel() -> Result<()> {
  use vpdf_ui::{
    keys::PanelKind,
    panel::{Panel, Row, help},
  };
  let rows = help(|_| false).iter().map(|line| Row::parse(line)).collect();
  let mut panel = Panel::new(PanelKind::Help, rows);
  let mut out = Vec::new();
  measure("Panel::list", || {
    out.clear();
    panel
      .list(&mut out, vpdf_terminal::terminal::At::default(), 50, 40)
      .expect("panel rows");
    black_box(&out);
  })
}

fn hints() -> Result<()> {
  use vpdf_model::document::Target;
  use vpdf_terminal::{graphics::Graphics, terminal::At};
  use vpdf_ui::hints::{Hints, Spot};
  use vpdf_ui::paint::Frame;
  #[derive(serde::Deserialize)]
  struct Prepared {
    screen: [u16; 4],
    found: Vec<(Spot, Target)>,
  }
  let prepared: Prepared = fixture("hints")?;
  let [columns, rows, cell_width, cell_height] = prepared.screen;
  let screen =
    Screen { columns, rows, cell_width, cell_height, origin: (0, 0) };
  let hints = Hints::new(prepared.found, false, 0, columns)?;
  let mut graphics = Graphics::default();
  let mut out = Vec::new();
  let frame = || Frame { at: At::default(), screen, left: 0, text: false };
  hints.draw(&mut graphics, &mut out, frame())?;
  measure("Hints::draw", || {
    out.clear();
    graphics.begin(&mut out).expect("begin hint frame");
    hints.draw(&mut graphics, &mut out, frame()).expect("prepared labels");
    graphics.end(&mut out).expect("hint placements");
    black_box(&out);
  })
}

fn main() -> Result<()> {
  for (name, case) in [
    ("key prefixes", key_prefixes as fn() -> Result<()>),
    ("projection", projection),
    ("page tiles", tiles::draw),
    ("page navigation", page::navigation),
    ("reading scroll", reading::scroll),
    ("reading navigation", reading::navigation),
    ("section candidates", sections::select),
    ("reading jumps", jumps::land),
    ("gesture selection", gestures::selection),
    ("text_canvas", text::canvas),
    ("cursorline", || overlays::draw("cursorline")),
    ("landing", || overlays::draw("landing")),
    ("link hover", || overlays::draw("link")),
    ("draw_hints", hints),
    ("context draw", context::draw),
    ("clues draw", clues::draw),
    ("region draw", region::draw),
    ("scrollbar draw", scrollbar::draw),
    ("route", route),
    ("routing panel", panel),
  ] {
    if support::selected(name) {
      case()?;
    }
  }
  Ok(())
}
