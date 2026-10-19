use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::hint::black_box;
use std::path::PathBuf;
use vpdf_model::geometry::Screen;
use vpdf_model::text::{Bounds, PageId};
use vpdf_ui::layout::{Layout, visible};
#[path = "../../../benches/support/hash.rs"]
mod hash;
#[path = "../../../benches/support/mod.rs"]
mod support;
use support::measure;

#[derive(serde::Deserialize)]
struct Manifest {
  geometry_sha256: String,
  source_sha256: String,
  revision: String,
}

#[derive(serde::Deserialize)]
struct Geometry {
  pages: Vec<[f32; 4]>,
  glyph: [f32; 4],
}

fn bounds([x0, y0, x1, y1]: [f32; 4]) -> Bounds {
  Bounds { x0, y0, x1, y1 }
}

fn fixture() -> Result<Geometry> {
  let directory = std::env::var_os("VPDF_BENCH_FIXTURES").map_or_else(
    || {
      PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/bench-fixtures")
    },
    PathBuf::from,
  );
  let manifest: Manifest = serde_json::from_slice(
    &std::fs::read(directory.join("manifest.json"))
      .context("Run mise run bench:fixtures first")?,
  )?;
  let bytes = std::fs::read(directory.join("geometry.json"))?;
  anyhow::ensure!(
    hash::digest(&bytes) == manifest.geometry_sha256,
    "Geometry fixture hash mismatch"
  );
  println!(
    "fixture {} source {} revision {}",
    manifest.geometry_sha256, manifest.source_sha256, manifest.revision
  );
  Ok(serde_json::from_slice(&bytes)?)
}

fn projection() -> Result<()> {
  let geometry = fixture()?;
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
    layout.zoom = zoom;
    layout.pan = 96.0;
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

fn main() -> Result<()> {
  for (name, case) in [
    ("key prefixes", key_prefixes as fn() -> Result<()>),
    ("projection", projection),
    ("route", route),
    ("routing panel", panel),
  ] {
    if support::selected(name) {
      case()?;
    }
  }
  Ok(())
}
