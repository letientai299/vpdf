use crate::Result;
use crate::document::Document;
use crate::keys::{Keys, PanelKind};
use crate::layout::{Layout, Screen};
use crate::raster::Raster;
use crate::text::Position;
use crate::tiles::TileKey;
use crate::window::{TabMove, Window};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;
use vpdf_reader::reader::Reader;
use vpdf_terminal::graphics::Usage;
use vpdf_terminal::terminal;

/// Work a tab hands to the session.
#[derive(Debug, PartialEq)]
pub enum Request {
  Open {
    path: PathBuf,
    tab: bool,
  },
  PanelOpen {
    path: PathBuf,
    position: Option<Position>,
    focus: bool,
  },
  PanelMark {
    name: char,
    focus: bool,
  },
  Tab(TabMove),
  /// A new tab listing recent documents over this one.
  RecentTab,
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
  /// A new split lands here; focus stays.
  SplitAt(Position),
  /// A new split follows a link there; focus stays.
  SplitTo(crate::document::Target),
  /// A global mark beside; focus stays.
  SplitMark(char),
  /// Keys a clicked clue names.
  Keys(Vec<crossterm::event::KeyEvent>),
  /// Pick a file with `fzf` and open it in the focused pane.
  Pick,
  /// Closes a tab, else the active one.
  TabClose(Option<usize>),
  QuitAll,
  /// `:mouse`: takes the pointer, or leaves it to the terminal.
  Mouse(bool),
  Perf(Option<bool>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputOwner {
  Prompt,
  Hints,
  Context,
  Peek,
  Region,
  Panel,
  Overview,
  Content,
}

/// File names of every tab and the active index.
#[derive(Default)]
pub struct Tabline {
  pub names: Vec<String>,
  pub active: usize,
}

#[expect(clippy::struct_excessive_bools, reason = "independent pane flags")]
pub struct App {
  pub id: u32,
  pub doc: Document,
  pub reader: Reader,
  pub target: Option<Position>,
  pub landing: Option<crate::marks::Landing>,
  /// Last landed view, for `]]`.
  pub(crate) landed: Option<crate::marks::Landed>,
  /// The followed target of a landed view.
  pub(crate) aimed: Option<(crate::marks::Landed, crate::document::Target)>,
  pub layout: Layout,
  /// Reloaded since the session last drew: its old images go first.
  pub(crate) images_stale: bool,
  /// The shared images as this pane last drew them.
  pub(crate) image_usage: Usage,
  pub worker: Raster,
  pub keys: Keys,
  pub path: PathBuf,
  pub password: String,
  /// File modification time and size at the last load.
  pub stamp: Option<crate::watch::Stamp>,
  /// A changed stamp and when it was first seen.
  pub(crate) settling: Option<(crate::watch::Stamp, Instant)>,
  pub text_mode: bool,
  pub cursorline: bool,
  /// Text origin; glyph stores column offset.
  pub(crate) text_top: Position,
  /// Text view origin on screen.
  pub(crate) text_drawn: Option<Position>,
  pub screen: Screen,
  pub left: u16,
  /// Side panel columns a drag chose.
  pub panel_width: Option<u16>,
  /// Pane origin on the terminal.
  pub at: terminal::At,
  pub quit: bool,
  /// Whether this pane has the keyboard.
  pub focused: bool,
  pub request: Option<Request>,
  pub tabline: Tabline,
  pub error: Option<String>,
  /// The message the pointer last moved past.
  pub render_ms: f64,
  pub search: crate::find::Search,
  pub search_worker: Option<vpdf_pdf::jobs::search::SearchWorker>,
  /// Wakes the event loop when a job finishes.
  pub wake: vpdf_pdf::jobs::Wake,
  /// Outline entry containing the cursor, refreshed each frame.
  pub current_section: Option<usize>,
  /// Lowercase marks of this document.
  pub marks: std::collections::BTreeMap<char, Position>,
  /// Where an open search prompt started.
  pub search_origin: Option<vpdf_reader::reader::Location>,
  pub prompt: Option<crate::prompt::Prompt>,
  pub hints: Option<crate::hints::Hints>,
  pub clues: Option<crate::clues::Clues>,
  /// When the pending prefix started waiting for its next key.
  pub awaiting: Option<Instant>,
  /// Link destination shown by `K`.
  pub peek: Option<crate::preview::Preview>,
  /// The right-click menu.
  pub context_menu: Option<crate::context::ContextMenu>,
  pub(crate) link_press: crate::links::LinkPress,
  /// What the held left button does.
  pub(crate) gesture: crate::gestures::Gesture,
  /// The held middle button.
  pub(crate) grab: Option<crate::gestures::Grab>,
  /// A drag held past an edge.
  pub(crate) autoscroll: Option<crate::gestures::Autoscroll>,
  pub region: Option<crate::region::Region>,
  pub region_job: Option<crate::region::Job>,
  pub region_send: Option<crate::region::Transfer>,
  pub overview: Option<crate::overview::Overview>,
  pub panel: Option<crate::panel::Panel>,
  pub preview: Option<bool>,
  pub ocr: Option<vpdf_pdf::jobs::ocr::OcrJob>,
  pub outline_load: crate::outline::Loading,
  pub crop_job: Option<vpdf_pdf::jobs::crop::Job>,
  pub animation: Option<crate::animation::Scroll>,
  /// The current wheel gesture over the page.
  pub(crate) wheel: Option<crate::interaction::WheelRun>,
  pub(crate) requested: Vec<TileKey>,
  /// Sign of the last scroll; prefetch looks that way first.
  pub(crate) heading: f32,
  /// The pane's rows above the status line are known blank.
  pub(crate) blank: bool,
  /// What the pointer rests on here.
  pub hover: crate::pointer::Hit,
  /// The page link under the pointer.
  pub(crate) rest: Option<crate::links::Rest>,
  /// The last frame showed the whole view: every visible tile placed.
  pub complete: bool,
  pub(crate) bar: crate::scrollbar::Bar,
}

impl App {
  pub fn name(&self) -> String {
    let name = self.path.file_name().map_or_else(
      || self.path.display().to_string(),
      |name| name.to_string_lossy().into_owned(),
    );
    crate::text::printable(&name).collect()
  }

  pub fn with_screen(
    path: &Path,
    password: &str,
    text_mode: bool,
    screen: Screen,
  ) -> Result<Self> {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let doc = Document::open(path, password)?;
    let layout = Layout::new(doc.pages.iter().map(|page| page.bounds), screen);
    let wake = vpdf_pdf::jobs::Wake::new(vpdf_terminal::input::wake);
    let worker = Raster::new(
      path,
      password,
      doc.lists(),
      &wake,
      vpdf_terminal::payload::Encoder::live(),
    );
    Ok(Self {
      id: NEXT.fetch_add(1, Ordering::Relaxed),
      doc,
      reader: Reader::default(),
      target: None,
      landing: None,
      landed: None,
      aimed: None,
      layout,
      images_stale: false,
      image_usage: Usage::default(),
      worker,
      keys: Keys::default(),
      path: path.to_owned(),
      password: password.to_owned(),
      stamp: crate::watch::stamp(path),
      settling: None,
      text_mode,
      cursorline: true,
      text_top: Position::default(),
      text_drawn: None,
      screen,
      left: 0,
      panel_width: None,
      at: terminal::At::default(),
      quit: false,
      focused: true,
      request: None,
      tabline: Tabline::default(),
      error: None,
      render_ms: 0.0,
      search: crate::find::Search::default(),
      search_worker: None,
      wake,
      current_section: None,
      marks: std::collections::BTreeMap::new(),
      search_origin: None,
      prompt: None,
      hints: None,
      clues: None,
      awaiting: None,
      peek: None,
      context_menu: None,
      link_press: crate::links::LinkPress::Idle,
      gesture: crate::gestures::Gesture::Glyphs,
      grab: None,
      autoscroll: None,
      region: None,
      region_job: None,
      region_send: None,
      overview: None,
      panel: None,
      preview: None,
      ocr: None,
      outline_load: crate::outline::Loading::Deferred,
      crop_job: None,
      animation: None,
      wheel: None,
      requested: Vec::new(),
      heading: 1.0,
      blank: false,
      hover: crate::pointer::Hit::Nothing,
      rest: None,
      complete: false,
      bar: crate::scrollbar::Bar::default(),
    })
  }
}
