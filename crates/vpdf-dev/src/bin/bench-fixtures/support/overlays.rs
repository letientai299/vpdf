use super::reader_fixture::SeedReader;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::Path;
use vpdf::presentation::try_draw;
use vpdf::solo::Solo;
use vpdf_model::geometry::Screen;
use vpdf_model::text::{PageId, Position};
use vpdf_reader::reader::SelectionKind;
use vpdf_ui::pointer::{Hit, Hover, Region};

fn point(app: &mut Solo, page: PageId, index: usize) {
  let id = app.id;
  app.point(Some(Hover {
    pane: id,
    region: Region::Page,
    hit: Hit::Link(page, index),
    beneath: Some((page, index)),
  }));
}

fn capture(app: &mut Solo, name: &str, source: &str) -> Result<Value> {
  app.graphics = vpdf_terminal::graphics::Graphics::default();
  let screen = app.layout.screen();
  let hovered = app.hover_segments();
  let landing = app.reader.landing().position();
  let left = app.left;
  let top = if app.text_mode {
    app.text().position(left, 0)?
  } else {
    Position::default()
  };
  let mut hashes = Vec::new();
  for _ in 0..2 {
    let mut out = Vec::new();
    if app.text_mode {
      try_draw(&mut out, None, |s| app.scene_text(s))?;
    } else {
      app.graphics.begin(&mut out)?;
      let (app, graphics) = app.parts();
      try_draw(&mut out, Some(graphics), |s| app.scene_marks(s, screen))?;
      graphics.end(&mut out)?;
    }
    hashes.push(super::hash::digest(&out));
  }
  Ok(json!({
    "name": name,
    "source": source,
    "text_mode": app.text_mode,
    "screen":
      [screen.columns, screen.rows, screen.cell_width, screen.cell_height],
    "origin": screen.origin,
    "at": [app.at.column, app.at.row],
    "left": app.left,
    "scroll": app.reader.scroll,
    "cursor": app.reader.cursor,
    "selection": app.reader.selection
      .map(|s| (s.anchor, s.kind == SelectionKind::Line)),
    "cursor_mode": app.reader.uses_cursor(),
    "top": top,
    "landing": landing,
    "cursorline": app.cursorline,
    "hovered": hovered.into_iter()
      .map(|(p, b)| (p, super::text::coordinates(b)))
      .collect::<Vec<_>>(),
    "output_sha256": hashes,
  }))
}

pub fn prepare(source: &Path, book: &Path, screen: Screen) -> Result<Vec<u8>> {
  let mut cases = Vec::new();
  for text_mode in [false, true] {
    let mut app = Solo::open(source, "", text_mode, screen)?;
    app.reader.seed_reader(|reader| reader.cursor.line = 3);
    app.reader.visual(SelectionKind::Character);
    cases.push(capture(&mut app, "cursorline", "text")?);
    let mut app = Solo::open(source, "", text_mode, screen)?;
    app.go_to(Position { line: 3, ..Position::default() }, true)?;
    cases.push(capture(&mut app, "landing", "text")?);
    let mut app = Solo::open(source, "", text_mode, screen)?;
    app.command("2")?;
    cases.push(capture(&mut app, "no link", "text")?);
    point(&mut app, PageId(1), 0);
    anyhow::ensure!(app.hover_link().is_some(), "Missing citation");
    cases.push(capture(&mut app, "link tint", "text")?);
  }
  let mut app = Solo::open(book, "", false, screen)?;
  app.command("4")?;
  let page = PageId(3);
  let index = app
    .doc
    .links(page)?
    .iter()
    .position(|link| {
      matches!(&link.target, vpdf_model::document::Target::Uri(uri)
      if uri.contains("greenteapress"))
    })
    .context("Missing wrapped address")?;
  point(&mut app, page, index);
  cases.push(capture(&mut app, "link tint address", "book")?);
  Ok(serde_json::to_vec(&cases)?)
}
