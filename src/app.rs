use crate::Result;
use crate::document::Document;
use crate::graphics::Graphics;
use crate::keys::Keys;
use crate::layout::{Layout, Screen};
use crate::reader::Reader;
use crate::render_worker::RenderWorker;
use crate::terminal;
use crate::text::Position;
use crate::tiles::TileKey;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

/// Work a tab hands to the session.
#[derive(Debug, Eq, PartialEq)]
pub enum Request {
  Open { path: PathBuf, tab: bool },
  Tab(TabMove),
  Recent,
  QuitAll,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TabMove {
  Next(usize),
  Previous(usize),
  Go(usize),
}

/// File names of every tab and the active index.
#[derive(Default)]
pub struct Tabline {
  pub names: Vec<String>,
  pub active: usize,
}

pub struct App {
  pub id: u32,
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
  pub request: Option<Request>,
  pub tabline: Tabline,
  pub error: Option<String>,
  pub render_ms: f64,
  pub search: crate::find::Search,
  pub prompt: Option<crate::prompt::Prompt>,
  pub panel: Option<crate::panel::Panel>,
  pub preview: Option<bool>,
  pub ocr: Option<crate::ocr::OcrJob>,
  pub animation: Option<crate::animation::Scroll>,
  pub(crate) requested: Vec<TileKey>,
  pub(crate) last_click: Option<(Instant, Position)>,
}

impl App {
  pub fn name(&self) -> String {
    self.path.file_name().map_or_else(
      || self.path.display().to_string(),
      |name| name.to_string_lossy().into_owned(),
    )
  }

  pub fn new(path: &Path, password: &str, text_mode: bool) -> Result<Self> {
    Self::with_screen(path, password, text_mode, terminal::size()?)
  }

  pub fn with_screen(
    path: &Path,
    password: &str,
    text_mode: bool,
    screen: Screen,
  ) -> Result<Self> {
    let doc = Document::open(path, password)?;
    let layout = Layout::new(&doc.pages, screen);
    static NEXT: AtomicU32 = AtomicU32::new(1);
    Ok(Self {
      id: NEXT.fetch_add(1, Ordering::Relaxed),
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
      request: None,
      tabline: Tabline::default(),
      error: None,
      render_ms: 0.0,
      search: crate::find::Search::default(),
      prompt: None,
      panel: None,
      preview: None,
      ocr: None,
      animation: None,
      requested: Vec::new(),
      last_click: None,
    })
  }
}
