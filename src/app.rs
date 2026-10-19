use crate::Result;
use crate::document::Document;
use crate::raster::Raster;
use crate::text::Position;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;
use vpdf_terminal::graphics::Usage;
use vpdf_terminal::terminal;
use vpdf_ui::keys::{Keys, PanelKind};
use vpdf_ui::layout::{Layout, Screen};
use vpdf_ui::reading::Pane;
use vpdf_ui::window::{TabMove, Window};

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
  pub reader: Pane,
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
  /// Pending and drawn text frames.
  pub(crate) text_view: vpdf_ui::text_view::TextView,
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
  pub search: vpdf_ui::find::Search,
  pub search_worker: Option<vpdf_pdf::jobs::search::SearchWorker>,
  /// Wakes the event loop when a job finishes.
  pub wake: vpdf_pdf::jobs::Wake,
  /// Outline entry containing the cursor, refreshed each frame.
  pub current_section: Option<usize>,
  /// Lowercase marks of this document.
  pub marks: std::collections::BTreeMap<char, Position>,
  /// Where an open search prompt started.
  pub search_origin: Option<vpdf_reader::reader::Location>,
  pub prompt: Option<vpdf_ui::prompt::Prompt>,
  pub hints: Option<vpdf_ui::hints::Hints>,
  pub clues: vpdf_ui::clues::Clues,
  /// Link destination shown by `K`.
  pub peek: Option<vpdf_ui::preview::Preview>,
  /// The right-click menu.
  pub context_menu: Option<vpdf_ui::context::ContextMenu>,
  pub(crate) link_press: vpdf_ui::input::links::Press,
  /// What the held left button does.
  pub(crate) gesture: vpdf_ui::input::gesture::Gesture,
  /// The held middle button.
  pub(crate) grab: vpdf_ui::input::grab::Grab,
  /// A drag held past an edge.
  pub(crate) autoscroll: vpdf_ui::input::edge::Edge,
  pub region: Option<vpdf_ui::region::Region>,
  pub region_job: Option<crate::region::Job>,
  pub region_send: Option<crate::region::Transfer>,
  pub overview: Option<vpdf_ui::overview::Overview>,
  pub panel: Option<crate::panel::PanelSession>,
  pub preview: Option<bool>,
  pub ocr: Option<vpdf_pdf::jobs::ocr::OcrJob>,
  pub outline_load: crate::outline::Loading,
  pub crop_scan: vpdf_pdf::jobs::crop::Scan,
  /// The current wheel gesture over the page.
  pub(crate) wheel: vpdf_ui::input::wheel::Wheel,
  pub(crate) tiles: vpdf_ui::layout::tiles::Tiles,
  /// The pane's rows above the status line are known blank.
  pub(crate) blank: bool,
  /// What the pointer rests on here.
  pub hover: vpdf_ui::input::links::Hover,
  /// The last frame showed the whole view: every visible tile placed.
  pub complete: bool,
  pub(crate) bar: vpdf_ui::scrollbar::Bar,
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
      reader: Pane::default(),
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
      text_view: vpdf_ui::text_view::TextView::default(),
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
      search: vpdf_ui::find::Search::default(),
      search_worker: None,
      wake,
      current_section: None,
      marks: std::collections::BTreeMap::new(),
      search_origin: None,
      prompt: None,
      hints: None,
      clues: vpdf_ui::clues::Clues::default(),
      peek: None,
      context_menu: None,
      link_press: vpdf_ui::input::links::Press::default(),
      gesture: vpdf_ui::input::gesture::Gesture::default(),
      grab: vpdf_ui::input::grab::Grab::default(),
      autoscroll: vpdf_ui::input::edge::Edge::default(),
      region: None,
      region_job: None,
      region_send: None,
      overview: None,
      panel: None,
      preview: None,
      ocr: None,
      outline_load: crate::outline::Loading::Deferred,
      crop_scan: vpdf_pdf::jobs::crop::Scan::default(),
      wheel: vpdf_ui::input::wheel::Wheel::default(),
      tiles: vpdf_ui::layout::tiles::Tiles::default(),
      blank: false,
      hover: vpdf_ui::input::links::Hover::default(),
      complete: false,
      bar: vpdf_ui::scrollbar::Bar::default(),
    })
  }
}
