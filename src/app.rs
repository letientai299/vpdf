//! A pane: its document, jobs, and UI state; open, reload, and trim.

use crate::Result;
use crate::raster::{Frontend, Raster};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;
use vpdf_model::geometry::At;
use vpdf_model::text::Position;
use vpdf_pdf::document::Document;
use vpdf_terminal::graphics::Usage;
use vpdf_ui::keys::{Keys, PanelKind};
use vpdf_ui::layout::{Layout, Screen};
use vpdf_ui::reading::Pane;
use vpdf_ui::window::{TabMove, Window};

/// Work a tab hands to the session.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
  HelpBeside {
    topic: String,
    invocation: vpdf_ui::topics::Invocation,
  },
  Help {
    topic: String,
    invocation: vpdf_ui::topics::Invocation,
  },
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
  ForgetRecent(Vec<PathBuf>),
  /// Removes a local mark, or a global one for every document.
  RemoveMark(char),
  /// A panel whose rows the session builds.
  Panel(PanelKind),
  SetMark(char),
  /// A local mark was set; open Marks panels refill.
  MarkSet,
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
  SplitTo(vpdf_model::document::Target),
  /// A global mark beside; focus stays.
  SplitMark(char),
  /// Keys a clicked clue names.
  Keys(Vec<vpdf_model::input::KeyEvent>),
  /// Pick a PDF for this destination.
  Pick {
    root: PathBuf,
    tab: bool,
  },
  Print,
  /// Closes a tab, else the active one.
  TabClose(Option<usize>),
  QuitAll,
  ConfigReload,
  Theme(vpdf_model::theme::Theme),
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
  pub document_generation: u64,
  retired_jobs: Vec<vpdf_pdf::jobs::Activity>,
  pub doc: Document,
  pub reader: Pane,
  pub layout: Layout,
  /// Reloaded since the session last drew: its old images go first.
  pub(crate) images_stale: bool,
  /// The shared images as this pane last drew them.
  pub(crate) image_usage: Usage,
  pub worker: Raster,
  pub(crate) frontend: Frontend,
  pub keys: Keys,
  pub path: PathBuf,
  pub password: String,
  /// File modification time and size at the last load.
  pub stamp: Option<crate::watch::Stamp>,
  /// A changed stamp and when it was first seen.
  pub(crate) settling: Option<(crate::watch::Stamp, Instant)>,
  pub text_mode: bool,
  pub cursorline: bool,
  pub(crate) settings: crate::config::Settings,
  pub(crate) region_notice: Option<String>,
  /// Pending and drawn text frames.
  pub(crate) text_view: vpdf_ui::text_view::TextView,
  pub screen: Screen,
  pub left: u16,
  /// Side panel columns a drag chose.
  pub panel_width: Option<u16>,
  /// Pane origin on the terminal.
  pub at: At,
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
  pub outline: vpdf_ui::reading::Outline,
  /// Lowercase marks of this document.
  pub marks: std::collections::BTreeMap<char, Position>,
  pub(crate) opened: crate::state::Visit,
  pub overlays: vpdf_ui::overlays::Overlays,
  pub(crate) link_press: vpdf_ui::input::links::Press,
  /// What the held left button does.
  pub(crate) gesture: vpdf_ui::input::gesture::Gesture,
  /// The held middle button.
  pub(crate) grab: vpdf_ui::input::grab::Grab,
  /// A drag held past an edge.
  pub(crate) autoscroll: vpdf_ui::input::edge::Edge,
  pub region_job: Option<crate::region::Job>,
  pub region_send: Option<crate::region::Transfer>,
  pub panel: Option<crate::panel::PanelSession>,
  pub preview: Option<bool>,
  pub ocr: Option<vpdf_pdf::jobs::ocr::OcrJob>,
  pub(crate) auto_ocr: crate::jobs::AutoOcr,
  pub outline_load: crate::jobs::Loading,
  pub figures: crate::jobs::Figures,
  pub(crate) sidebar: crate::sidebar::Sidebar,
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
  pub(crate) manual: Option<std::sync::Arc<vpdf_pdf::manual::Document>>,
}

impl App {
  pub(crate) fn busy(&self) -> bool {
    self.reader.animation().active()
      || self.search.phase().running()
      || self.crop_scan.active()
      || matches!(self.outline_load, crate::jobs::Loading::Running(_))
      || matches!(self.figures, crate::jobs::Figures::Running(_))
      || self.annotations_loading()
      || self.ocr.is_some()
  }

  pub fn name(&self) -> String {
    if self.manual.is_some() {
      return "Help".into();
    }
    let name = self.path.file_name().map_or_else(
      || self.path.display().to_string(),
      |name| name.to_string_lossy().into_owned(),
    );
    vpdf_model::text::printable(&name).collect()
  }

  pub fn with_screen(
    path: &Path,
    password: &str,
    text_mode: bool,
    screen: Screen,
  ) -> Result<Self> {
    let settings = crate::config::Settings::default();
    Self::configured(
      path,
      password,
      text_mode,
      screen,
      Frontend::Terminal,
      &settings,
    )
  }

  pub(crate) fn configured(
    path: &Path,
    password: &str,
    text_mode: bool,
    screen: Screen,
    frontend: Frontend,
    settings: &crate::config::Settings,
  ) -> Result<Self> {
    Self::load_document(
      path, password, text_mode, screen, frontend, settings, None,
    )
  }

  pub(crate) fn help_document(
    document: std::sync::Arc<vpdf_pdf::manual::Document>,
    screen: Screen,
    frontend: Frontend,
    settings: &crate::config::Settings,
  ) -> Result<Self> {
    let path = document.path().to_owned();
    let mut app = Self::load_document(
      &path,
      "",
      false,
      screen,
      frontend,
      settings,
      Some(document),
    )?;
    app.text_view.set_wrap(true);
    Ok(app)
  }

  fn load_document(
    path: &Path,
    password: &str,
    text_mode: bool,
    screen: Screen,
    frontend: Frontend,
    settings: &crate::config::Settings,
    manual: Option<std::sync::Arc<vpdf_pdf::manual::Document>>,
  ) -> Result<Self> {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let doc = Document::open(path, password)?;
    let mut layout =
      Layout::new(doc.pages.iter().map(|page| page.bounds), screen);
    if settings.zoom().to_bits() != layout.zoom().to_bits() {
      layout.request_zoom(vpdf_ui::layout::Zoom::To(settings.zoom()));
      layout.reflow(doc.pages.iter().map(|page| page.bounds));
    }
    let wake = frontend.wake();
    let wake = if let Some(owner) = manual.clone() {
      vpdf_pdf::jobs::Wake::new(move || {
        let _owner = &owner;
        wake.wake();
      })
    } else {
      wake
    };
    let worker = frontend.raster(path, password, doc.lists(), &wake, false);
    Ok(Self {
      id: NEXT.fetch_add(1, Ordering::Relaxed),
      document_generation: 0,
      retired_jobs: Vec::new(),
      outline: vpdf_ui::reading::Outline::new(&doc.outline),
      doc,
      reader: Pane::default(),
      layout,
      images_stale: false,
      image_usage: Usage::default(),
      worker,
      frontend,
      keys: Keys::default(),
      path: path.to_owned(),
      password: password.to_owned(),
      stamp: crate::watch::stamp(path),
      settling: None,
      text_mode,
      cursorline: settings.cursorline(),
      settings: settings.clone(),
      region_notice: None,
      text_view: vpdf_ui::text_view::TextView::default(),
      screen,
      left: 0,
      panel_width: None,
      at: At::default(),
      quit: false,
      focused: true,
      request: None,
      tabline: Tabline::default(),
      error: None,
      render_ms: 0.0,
      search: vpdf_ui::find::Search::default(),
      search_worker: None,
      wake,
      marks: std::collections::BTreeMap::new(),
      opened: crate::state::Visit::default(),
      overlays: vpdf_ui::overlays::Overlays::default(),
      link_press: vpdf_ui::input::links::Press::default(),
      gesture: vpdf_ui::input::gesture::Gesture::default(),
      grab: vpdf_ui::input::grab::Grab::default(),
      autoscroll: vpdf_ui::input::edge::Edge::default(),
      region_job: None,
      region_send: None,
      panel: None,
      preview: None,
      ocr: None,
      auto_ocr: crate::jobs::AutoOcr::default(),
      outline_load: crate::jobs::Loading::Deferred,
      figures: crate::jobs::Figures::Deferred,
      sidebar: crate::sidebar::Sidebar::default(),
      crop_scan: vpdf_pdf::jobs::crop::Scan::default(),
      wheel: vpdf_ui::input::wheel::Wheel::default(),
      tiles: vpdf_ui::layout::tiles::Tiles::default(),
      blank: false,
      hover: vpdf_ui::input::links::Hover::default(),
      complete: false,
      bar: vpdf_ui::scrollbar::Bar::default(),
      manual,
    })
  }
}

impl App {
  pub(crate) fn track_jobs(&mut self) {
    self.retired_jobs.retain(vpdf_pdf::jobs::Activity::active);
    let mut activities = vec![self.worker.service.activity()];
    if let Some(worker) = &self.search_worker {
      activities.push(worker.activity());
    }
    for activity in activities {
      activity.observe(&self.wake);
      if activity.active()
        && !self.retired_jobs.iter().any(|old| old.same(&activity))
      {
        self.retired_jobs.push(activity);
      }
    }
  }

  pub(crate) fn active_workers(&self) -> usize {
    let current = self.worker.service.activity();
    let search = self
      .search_worker
      .as_ref()
      .map(vpdf_pdf::jobs::search::SearchWorker::activity);
    let mut activities = self.retired_jobs.clone();
    for activity in std::iter::once(current).chain(search) {
      if !activities.iter().any(|old| old.same(&activity)) {
        activities.push(activity);
      }
    }
    activities.iter().filter(|a| a.active()).count()
  }

  /// Releases what a hidden pane rebuilds on demand: the search worker
  /// with its indexes, cached text and display lists, and the render
  /// coordinator's document with every page it loaded. A running search
  /// resumes when the pane shows again.
  pub fn trim(&mut self) {
    self.crop_scan.clear();
    self.track_jobs();
    self.search_worker = None;
    self.doc.trim();
    self.worker = self.frontend.raster(
      &self.path,
      &self.password,
      self.doc.lists(),
      &self.wake,
      true,
    );
    self.tiles.invalidate();
  }

  /// Reopens the document, keeping the view where it still applies.
  pub fn reload_with(&mut self, password: &str) -> Result<()> {
    anyhow::ensure!(
      self.manual.is_none(),
      "Help reloads with applied configuration"
    );
    // A write during the open shows up as a later change.
    let stamp = crate::watch::stamp(&self.path);
    self.doc = Document::open(&self.path, password)?;
    self.refill_panel(PanelKind::Outline)?;
    self.document_generation += 1;
    self.crop_scan.clear();
    self.layout.clear_crops();
    self.layout.reflow(self.doc.pages.iter().map(|page| page.bounds));
    self.stamp = stamp;
    password.clone_into(&mut self.password);
    if !self.layout.crop().is_none() {
      self.start_crop();
    }
    self.track_jobs();
    self.worker = self.frontend.raster(
      &self.path,
      password,
      self.doc.lists(),
      &self.wake,
      false,
    );
    self.track_jobs();
    self.search_worker = None;
    let rerun = self.search.reset_document();
    // An open outline panel waits on the new outline now.
    self.outline_load = crate::jobs::Loading::Deferred;
    self.refresh_figures()?;
    self.refresh_sidebars()?;
    if self
      .panel
      .as_ref()
      .is_some_and(|panel| panel.view.kind() == PanelKind::Outline)
    {
      self.load_outline();
    }
    self.images_stale = true;
    if let Some(ocr) = self.ocr.take() {
      ocr.cancel();
    }
    self.auto_ocr = crate::jobs::AutoOcr::default();
    self.reader.reload();
    self.tiles.invalidate();
    self.reading_view().reclamp_cursor()?;
    self.text().reclamp()?;
    self.text_view.invalidate();
    self.point(None);
    self.link_press.cancel();
    self.dismiss_overlay();
    let mut reading = self.reading_parts();
    reading.view.reclamp_search(reading.search)?;
    if let Some(panel) = &self.panel {
      let kind = panel.view.kind();
      let rows = self.panel_rows(kind)?;
      // These rows come from the session or refill when work finishes.
      let refills = match kind {
        PanelKind::Recent | PanelKind::Marks => true,
        PanelKind::Results => rerun.is_some(),
        PanelKind::Outline => self.outline_pending(),
        PanelKind::Annotations => self.annotations_loading(),
        PanelKind::Figures => {
          matches!(self.figures, crate::jobs::Figures::Running(_))
        }
        _ => false,
      };
      if kind == PanelKind::Outline {
        self.refill_panel(kind)?;
      } else if self.panel.as_ref().is_some_and(|panel| {
        !rows.is_empty() || !refills && panel.view.explains_empty()
      }) {
        self.refill_rows(kind, rows);
      } else if !refills {
        self.panel = None;
      }
    }
    if let Some(query) = rerun {
      self.rerun(&query)?;
    }
    self.relayout()?;
    self.reader.notice("Reloaded");
    Ok(())
  }

  /// Copies another pane's view of the same document, as a new split.
  pub fn mirror(&mut self, other: &Self) -> Result<()> {
    self.crop_scan.clear();
    self.layout.copy_view(&other.layout);
    self.doc.share_margins(&other.doc);
    self.text_mode = other.text_mode;
    self.preview = other.preview;
    self.layout.reflow(self.doc.pages.iter().map(|page| page.bounds));
    let shown = other.navigation_position();
    self.reading_view().mirror(shown, other.reader.scroll, &other.text_view)?;
    if !self.layout.crop().is_none() && self.layout.crops().is_empty() {
      self.start_crop();
    }
    Ok(())
  }
}
