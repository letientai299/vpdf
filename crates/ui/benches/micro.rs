#[path = "../../../benches/support/directory.rs"]
mod directory;
#[path = "../../../benches/support/layout.rs"]
mod layouts;
use anyhow::Result;
use std::hint::black_box;
use vpdf_model::geometry::Screen;
use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};
use vpdf_model::text::PageId;
use vpdf_ui::layout::{Layout, visible};
mod content;
#[path = "../../../benches/support/fixtures.rs"]
mod fixtures;
mod gestures;
mod page;
mod preview;
#[path = "../../../benches/support/raster.rs"]
mod raster;
mod scenes;
use fixtures::{bounds, read as fixture};
#[path = "../../../benches/support/hash.rs"]
mod hash;
mod jumps;
mod links;
#[path = "../../../benches/support/pages.rs"]
mod pages;
mod reading;
mod search;
mod sections;
#[path = "../../../benches/support/mod.rs"]
mod support;
#[path = "../../../benches/support/text_source.rs"]
mod text_source;
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
  use vpdf_ui::keys::Grammar;
  for (name, input, grammar) in [
    ("Keys::feed navigation", "12j2gg", Grammar::Objects),
    ("Keys::feed sections", "2[[3]]", Grammar::Objects),
    ("Keys::feed panel yank", "\"a2yy", Grammar::Panel),
  ] {
    let events: Vec<_> = input
      .chars()
      .map(|ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
      .collect();
    measure(name, || {
      let mut keys = vpdf_ui::keys::Keys::default();
      for event in &events {
        black_box(keys.feed(black_box(*event), grammar));
      }
    })?;
  }
  Ok(())
}

/// A drag across a two-pane split.
fn route() -> Result<()> {
  use vpdf_model::input::{
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

fn overview() -> Result<()> {
  use vpdf_reader::motion::Motion;
  use vpdf_ui::keys::{Action, Grammar, Input, Keys};
  use vpdf_ui::overview::{Grid, Overview};
  let geometry: Geometry = fixture("geometry")?;
  let pages = geometry.pages.len();
  anyhow::ensure!(pages > 2, "Missing overview pages");
  let grid = Grid::new(
    Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    },
    bounds(geometry.pages[0]),
  );
  let mut overview = Overview::new(PageId(0));
  measure("Overview::navigate pair", || {
    for motion in [Motion::Right, Motion::Left] {
      let input = Input {
        action: Action::Motion(motion),
        count: 2,
        explicit: true,
        register: 'a',
      };
      black_box(overview.navigate(black_box(input), &grid, pages));
    }
  })?;
  let mut keys = Keys::default();
  keys.feed(
    KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
    Grammar::Other,
  );
  measure("Keys::clues overview", || {
    black_box(keys.clues(Grammar::Other, Overview::allows));
  })
}

fn lexical_objects() -> Result<()> {
  let prepared = fixtures::text()?;
  let text = &prepared.text[0];
  let line = text
    .lines
    .iter()
    .position(|line| line.glyphs.len() > 15)
    .ok_or_else(|| anyhow::anyhow!("Missing prose line"))?;
  let letters: Vec<_> = text.lines[line]
    .glyphs
    .iter()
    .filter(|g| g.ch.is_alphabetic())
    .take(8)
    .map(|g| KeyEvent::new(KeyCode::Char(g.ch), KeyModifiers::NONE))
    .collect();
  let mut keys = vpdf_ui::keys::Keys::default();
  measure("Keys::feed objects paper", || {
    keys.clear();
    for ch in "3iw".chars() {
      black_box(keys.feed(
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
        vpdf_ui::keys::Grammar::Objects,
      ));
    }
    for key in &letters {
      black_box(keys.feed(*key, vpdf_ui::keys::Grammar::Objects));
    }
  })
}

fn main() -> Result<()> {
  support::run(&[
    ("scene construction", scenes::compose),
    ("key prefixes", key_prefixes),
    ("lexical objects", lexical_objects),
    ("projection", projection),
    ("preview raster", preview::compose),
    ("page navigation", page::navigation),
    ("reading scroll", reading::scroll),
    ("reading viewport", reading::viewport),
    ("reading navigation", reading::navigation),
    ("section candidates", sections::select),
    ("reading jumps", jumps::land),
    ("search navigation", search::navigate),
    ("definition resolve", content::definition),
    ("gesture selection", gestures::selection),
    ("link state", links::transitions),
    ("panel content", content::rows),
    ("help reference", content::help),
    ("panel input", content::input),
    ("overview input", overview),
    ("outline tracking", content::tracking),
    ("route", route),
  ])
}
