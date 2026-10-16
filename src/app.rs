use crate::Result;
use crate::document::Document;
use crate::graphics::Graphics;
use crate::keys::Keys;
use crate::layout::{Layout, Screen};
use crate::reader::Reader;
use crate::render_worker::RenderWorker;
use crate::terminal::{self, Terminal};
use crate::text::{PageId, Position};
use crate::tiles::TileKey;
use crossterm::event::{self, Event, KeyEventKind};
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct App {
  pub doc: Document,
  pub reader: Reader,
  pub layout: Layout,
  pub graphics: Graphics,
  pub worker: RenderWorker,
  pub keys: Keys,
  pub path: PathBuf,
  pub password: String,
  pub text_mode: bool,
  pub screen: Screen,
  pub left: u16,
  pub quit: bool,
  pub render_ms: f64,
  pub(crate) requested: Vec<TileKey>,
  pub(crate) last_click: Option<(Instant, Position)>,
}

impl App {
  pub fn new(path: &Path, password: &str, text_mode: bool) -> Result<Self> {
    let doc = Document::open(path, password)?;
    let screen = terminal::size()?;
    let layout = Layout::new(&doc.pages, screen);
    Ok(Self {
      doc,
      reader: Reader::default(),
      layout,
      graphics: Graphics::default(),
      worker: RenderWorker::new(path, password),
      keys: Keys::default(),
      path: path.to_owned(),
      password: password.to_owned(),
      text_mode,
      screen,
      left: 0,
      quit: false,
      render_ms: 0.0,
      requested: Vec::new(),
      last_click: None,
    })
  }
}

pub fn view(
  path: &Path,
  password: &str,
  text: bool,
  page: usize,
) -> Result<()> {
  anyhow::ensure!(io::stdout().is_terminal(), "TUI requires a terminal");
  let mut app = App::new(path, password, text)?;
  let _terminal = Terminal::enter()?;
  app.reader.edge(
    PageId(page.saturating_sub(1).min(app.doc.pages.len() - 1)),
    false,
    &mut app.doc,
  )?;
  app.ensure_cursor()?;
  let mut out = io::BufWriter::new(io::stdout());
  let mut dirty = true;
  while !app.quit {
    dirty |= app.raster_updates(&mut out)?;
    if dirty {
      app.draw(&mut out)?;
      dirty = false;
    }
    if !event::poll(Duration::from_millis(8))? {
      continue;
    }
    let result = match event::read()? {
      Event::Key(key) if key.kind != KeyEventKind::Release => {
        if let Some(input) = app.keys.feed(key) {
          app.navigate(input).and_then(|done| {
            if !done {
              app.viewport(input)?;
            }
            Ok(())
          })
        } else {
          Ok(())
        }
      }
      Event::Resize(_, _) => app.resize(),
      Event::Mouse(mouse) => app.mouse(mouse),
      _ => Ok(()),
    };
    if let Err(error) = result {
      app.reader.message = error.to_string();
    }
    dirty = true;
  }
  Ok(())
}
