use crate::Result;
use crate::app::{App, Request};
use crate::recent::Loader;
use crate::state::Entry;
use crossterm::event::{MouseEvent, MouseEventKind};
use std::io::Write;
use std::path::PathBuf;
use vpdf_model::document::Target;
use vpdf_terminal::{
  graphics::Graphics,
  terminal::{self, At},
};
use vpdf_ui::keys::{Input, PanelKind};
use vpdf_ui::layout::Screen;
use vpdf_ui::panel::{Destination, Panel, Row, help};

pub struct PanelSession {
  pub view: Panel,
  loader: Option<Loader>,
}

impl PanelSession {
  pub fn new(kind: PanelKind, rows: Vec<Row>) -> Self {
    Self { view: Panel::new(kind, rows), loader: None }
  }

  pub fn recent(
    rows: Vec<Row>,
    entries: &[Entry],
    password: &str,
    wake: &vpdf_pdf::jobs::Wake,
  ) -> Self {
    let view = Panel::recent(rows, &crate::recent::entries(entries));
    Self { view, loader: Some(Loader::new(password, wake)) }
  }

  pub fn updates(&mut self) -> bool {
    match (self.loader.as_ref(), self.view.shelf_mut()) {
      (Some(loader), Some(cards)) => loader.updates(cards),
      _ => false,
    }
  }

  pub fn draw(
    &mut self,
    out: &mut impl Write,
    at: At,
    width: u16,
    screen: Screen,
    graphics: Option<&mut Graphics>,
  ) -> Result<()> {
    let drawn =
      self.view.draw(out, at, width, screen, graphics, crate::state::now());
    if let (Some(loader), Some(cards)) = (&self.loader, self.view.shelf_mut()) {
      loader.request(cards);
    }
    drawn
  }
}

mod actions;
mod sections;
pub use sections::recent_rows;
