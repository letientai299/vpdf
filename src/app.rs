use crate::Result;
use crate::document::Document;
use crate::graphics::Graphics;
use crate::keys::{Keys, PanelKind};
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
  Open {
    path: PathBuf,
    tab: bool,
  },
  /// A picked document: a new tab, or the focused pane when split.
  Choose(PathBuf),
  Tab(TabMove),
  /// A panel whose rows the session builds.
  Panel(PanelKind),
  SetMark(char),
  /// A pane command with its count; `file` opens in a new split.
  Window {
    command: Window,
    count: usize,
    explicit: bool,
    file: Option<PathBuf>,
  },
  GoMark(char, bool),
  /// Pick a file with `fzf` and open it in a new tab.
  Pick,
  QuitAll,
}

/// `Ctrl-w` commands on the panes of a tab.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Window {
  Split(crate::split::Axis),
  Focus(crate::split::Direction),
  Next,
  Previous,
  Last,
  Close,
  Only,
  Equalize,
  Resize(crate::split::Axis, i32),
  Maximize(crate::split::Axis),
  Exchange,
  /// Rotates panes in the focused row or column; `true` moves forward.
  Rotate(bool),
  /// Splits above and lists recent documents to open there.
  New,
  /// Moves the pane to the full-height or full-width edge.
  Move(crate::split::Direction),
  ToTab,
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
  /// File modification time and size at the last load.
  pub stamp: Option<crate::watch::Stamp>,
  /// A changed stamp and when it was first seen.
  pub(crate) settling: Option<(crate::watch::Stamp, Instant)>,
  pub text_mode: bool,
  pub screen: Screen,
  pub left: u16,
  /// Pane origin on the terminal.
  pub at: crate::terminal::At,
  pub quit: bool,
  /// Whether this pane has the keyboard.
  pub focused: bool,
  pub request: Option<Request>,
  pub tabline: Tabline,
  pub error: Option<String>,
  pub render_ms: f64,
  pub search: crate::find::Search,
  /// Outline entry containing the cursor, refreshed each frame.
  pub current_section: Option<usize>,
  /// Lowercase marks of this document.
  pub marks: std::collections::BTreeMap<char, Position>,
  /// Where an open search prompt started.
  pub search_origin: Option<crate::reader::Location>,
  pub prompt: Option<crate::prompt::Prompt>,
  pub hints: Option<crate::hints::Hints>,
  pub clues: Option<crate::clues::Clues>,
  /// When the pending prefix started waiting for its next key.
  pub awaiting: Option<Instant>,
  /// Link destination shown by `K`.
  pub peek: Option<crate::preview::Preview>,
  pub overview: Option<crate::overview::Overview>,
  pub panel: Option<crate::panel::Panel>,
  pub preview: Option<bool>,
  pub ocr: Option<crate::ocr::OcrJob>,
  pub outline_job:
    Option<std::sync::mpsc::Receiver<Vec<crate::document::Heading>>>,
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
    let worker = RenderWorker::new(path, password, doc.lists());
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let mut app = Self {
      id: NEXT.fetch_add(1, Ordering::Relaxed),
      doc,
      reader: Reader::default(),
      layout,
      graphics: Graphics::default(),
      worker,
      keys: Keys::default(),
      path: path.to_owned(),
      password: password.to_owned(),
      stamp: crate::watch::stamp(path),
      settling: None,
      text_mode,
      screen,
      left: 0,
      at: crate::terminal::At::default(),
      quit: false,
      focused: true,
      request: None,
      tabline: Tabline::default(),
      error: None,
      render_ms: 0.0,
      search: crate::find::Search::default(),
      current_section: None,
      marks: std::collections::BTreeMap::new(),
      search_origin: None,
      prompt: None,
      hints: None,
      clues: None,
      awaiting: None,
      peek: None,
      overview: None,
      panel: None,
      preview: None,
      ocr: None,
      outline_job: None,
      animation: None,
      requested: Vec::new(),
      last_click: None,
    };
    app.infer_outline();
    Ok(app)
  }
}
