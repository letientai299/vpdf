use anyhow::{Context, Result};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::path::Path;
use vpdf_model::geometry::Screen;
use vpdf_terminal::graphics::Graphics;

pub fn prepare(source: &Path, screen: Screen) -> Result<Vec<u8>> {
  let mut session = vpdf::session::Session::new(None);
  session.open(source, true, screen)?;
  session.app().command("2")?;
  let press = (40, 20);
  session.mouse(MouseEvent {
    kind: MouseEventKind::Down(MouseButton::Right),
    column: press.0,
    row: press.1,
    modifiers: KeyModifiers::NONE,
  })?;
  let app = session.app();
  let mut graphics = Graphics::default();
  let mut output = Vec::new();
  graphics.begin(&mut output)?;
  app.draw_context(&mut graphics, &mut output)?;
  graphics.end(&mut output)?;
  let menu = app.overlays.menu().context("No menu at fixture press")?;
  let area = menu.area();
  let screen = app.layout.screen();
  serde_json::to_vec(&serde_json::json!({
    "entries": menu.entries().iter().map(|item| (&item.label, &item.key)).collect::<Vec<_>>(),
    "press": press,
    "columns": [app.left, app.screen.columns],
    "bottom": app.screen.rows.saturating_sub(1 + app.menu_rows()),
    "area": [area.column, area.row, area.columns, area.rows],
    "screen": [screen.columns, screen.rows, screen.cell_width, screen.cell_height],
    "origin": screen.origin,
    "at": [app.at.column, app.at.row],
    "text": app.text_mode,
    "row": menu.is_row(),
    "output_sha256": super::hash::digest(&output),
  })).map_err(Into::into)
}
