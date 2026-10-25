#[path = "../../../benches/support/mod.rs"]
mod support;
use anyhow::Result;
use eframe::egui;
use std::hint::black_box;
use std::sync::Arc;
use vpdf::session::native::{Frame, Image, Metrics};
use vpdf_gui::paint::Renderer;
use vpdf_model::geometry::Screen;
use vpdf_model::scene::{Op, Stage, Text, TilePlacement};
use vpdf_model::style::Role;
use vpdf_model::text::PageId;
use vpdf_model::theme::Theme;
use vpdf_model::tile::{TileId, TileKey, TileOwner};

/// A 1680x2160 window of 10x20 cells.
const SCREEN: Screen = Screen {
  columns: 168,
  rows: 108,
  cell_width: 10,
  cell_height: 20,
  origin: (0, 0),
};

#[derive(Clone, Copy)]
enum Rows {
  /// Only the status line.
  Status,
  /// Every row erased, as with a panel open.
  Cleared,
  /// Every row filled with text, as in text mode.
  Text,
}

/// Cached page tiles under a status line and `rows`.
fn frame(rows: Rows) -> Frame {
  let owner = TileOwner::new(1);
  let stage = Stage { screen: SCREEN, left: 0 };
  let mut scene = vec![Op::Theme(Theme::Default), Op::Style(Role::Reset)];
  let line = "The quick brown fox jumps over the lazy dog. ".repeat(4);
  let fill = match rows {
    Rows::Status => None,
    Rows::Cleared => Some(Op::Erase(SCREEN.columns)),
    Rows::Text => Some(Op::Text(Text::new(line))),
  };
  for row in fill.iter().flat_map(|_| 0..usize::from(SCREEN.rows)) {
    scene.push(Op::Move { column: 0, row });
    scene.extend(fill.clone());
  }
  let mut images = Vec::new();
  for index in 0..8_u16 {
    let key = TileKey {
      page: PageId(usize::from(index / 4)),
      scale: 1024,
      x: i32::from(index % 4) * 64,
      width: 64,
      height: 64,
      ..TileKey::default()
    };
    let id = TileId { owner, key };
    let origin = (key.x as f32, f32::from(index / 4 * 64));
    scene.push(Op::Tile(Box::new(TilePlacement::new(id, origin, stage))));
    images.push(Image { id, pixels: Arc::new(vec![255; 64 * 64 * 3]) });
  }
  let status = format!("{:<150}{:>18}", " manual.pdf", "12/5364 ");
  scene.extend([
    Op::Move { column: 0, row: usize::from(SCREEN.rows) - 1 },
    Op::Style(Role::Bar),
    Op::Text(Text::new(status)),
  ]);
  Frame {
    metrics: Metrics::default(),
    content: true,
    screen: SCREEN,
    scene,
    images,
    owners: vec![owner],
    title: String::new(),
    cursor: vpdf_model::style::Shape::Default,
    complete: true,
    quit: false,
    clipboard: None,
    reloaded: Vec::new(),
  }
}

/// Overlay grids around cached tiles, above an empty egui pass.
fn native_paint() -> Result<()> {
  let ctx = egui::Context::default();
  support::measure("egui empty pass", || {
    black_box(ctx.run_ui(egui::RawInput::default(), |_| {}).shapes);
  })?;
  let mut renderer = Renderer::default();
  for (name, rows) in [
    ("Renderer::paint status", Rows::Status),
    ("Renderer::paint cleared", Rows::Cleared),
    ("Renderer::paint text", Rows::Text),
  ] {
    let frame = frame(rows);
    support::measure(name, || {
      let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        black_box(renderer.paint(ui.painter(), &frame, SCREEN, 1.0, None));
      });
      black_box(output.shapes);
    })?;
  }
  Ok(())
}

fn main() -> Result<()> {
  support::run(&[("native paint", native_paint)])
}
