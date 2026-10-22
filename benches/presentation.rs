#[path = "support/directory.rs"]
mod directory;
#[path = "support/layout.rs"]
mod layouts;
#[path = "../tests/support/presentation.rs"]
pub mod presentation;
use crate::presentation::{HintsDraw, PanelDraw};
use anyhow::Result;
use std::hint::black_box;
use vpdf_model::geometry::Screen;
use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};
#[path = "presentation/chrome/mod.rs"]
mod chrome;
#[path = "presentation/clues/mod.rs"]
mod clues;
#[path = "presentation/context/mod.rs"]
mod context;
#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "presentation/home/mod.rs"]
mod home;
#[path = "presentation/overlays/mod.rs"]
mod overlays;
#[path = "presentation/preview/mod.rs"]
mod preview;
use fixtures::{bounds, read as fixture};
#[path = "support/hash.rs"]
mod hash;
#[path = "support/pages.rs"]
mod pages;
#[path = "presentation/region/mod.rs"]
mod region;
#[path = "presentation/scrollbar/mod.rs"]
mod scrollbar;
#[path = "support/mod.rs"]
mod support;
#[path = "presentation/text/mod.rs"]
mod text;
#[path = "support/text_source.rs"]
mod text_source;
#[path = "presentation/tiles/mod.rs"]
mod tiles;
use support::measure;

#[derive(serde::Deserialize)]
struct Geometry {
  pages: Vec<[f32; 4]>,
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
      .list(&mut out, vpdf_model::geometry::At::default(), 50, 40)
      .expect("panel rows");
    black_box(&out);
  })
}

fn hints() -> Result<()> {
  use vpdf_model::document::Target;
  use vpdf_model::geometry::At;
  use vpdf_terminal::graphics::Graphics;
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
  let mut hints = Hints::new(prepared.found, false, 0, columns)?;
  measure("Hints::accepts batch", || {
    for key in [
      KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
      KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL),
      KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    ] {
      black_box(hints.accepts(black_box(key)));
    }
  })?;
  let first = hints.items()[0].label.chars().next().expect("hint label");
  measure("Hints::key pair", || {
    for code in [KeyCode::Char(first), KeyCode::Backspace] {
      black_box(hints.key(black_box(KeyEvent::new(code, KeyModifiers::NONE))));
    }
  })?;
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

fn help() -> Result<()> {
  use vpdf_model::geometry::At;
  use vpdf_ui::keys::PanelKind;
  use vpdf_ui::panel::{Panel, help};
  measure("panel::help reference", || {
    black_box(help(|_| true));
  })?;
  for width in [30, 64] {
    let rows = help(|_| true)
      .iter()
      .map(|line| vpdf_ui::panel::Row::parse(line))
      .collect();
    let mut panel = Panel::new(PanelKind::Help, rows);
    let mut out = Vec::new();
    panel.list(&mut out, At::default(), width, 24)?;
    println!("help {width}x24 output {}", hash::digest(&out));
    measure(&format!("Panel::help draw {width}x24"), || {
      out.clear();
      panel.list(&mut out, At::default(), width, 24).expect("help frame");
      black_box(&out);
    })?;
  }
  Ok(())
}

fn main() -> Result<()> {
  support::run(&[
    ("page tiles", tiles::draw),
    ("preview draw", preview::draw),
    ("text_canvas", text::canvas),
    ("cursorline", || overlays::draw("cursorline")),
    ("landing", || overlays::draw("landing")),
    ("link hover", || overlays::draw("link")),
    ("draw_hints", hints),
    ("context draw", context::draw),
    ("home draw", home::draw),
    ("clues draw", clues::draw),
    ("region draw", region::draw),
    ("scrollbar draw", scrollbar::draw),
    ("routing panel", panel),
    ("routing status", chrome::draw),
    ("help reference", help),
  ])
}
