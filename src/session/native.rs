//! Session ownership for native windows.
use super::Pane;
use super::interaction::Interaction;
use super::maintenance::Chores;
use super::unlock::Operation;
use super::{Screen, Session};
use crate::Result;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use vpdf_model::cache::Cache;
use vpdf_model::input::{KeyEvent, MouseEvent};
use vpdf_model::scene::{Op, Recorder, Scene};
use vpdf_model::tile::{TileId, TileOwner};
use vpdf_pdf::jobs::Wake;

pub mod commands;
pub mod restore;

pub use super::pointer::Gesture;
pub use vpdf_ui::input::wheel::Precise;

const BUDGET: usize = 64 << 20;
/// Inputs one frame takes beyond the first, so a stream cannot hold it.
const BATCH: usize = 64;

#[derive(Default, Eq, PartialEq)]
pub(super) enum Motion {
  #[default]
  Full,
  Reduced,
}

pub enum Input {
  Action(commands::Command),
  Picked {
    request: commands::RequestId,
    paths: Vec<PathBuf>,
  },
  PasteTarget {
    command: commands::Command,
    text: String,
  },
  /// A one-based page for `GoToPage`.
  Page {
    command: commands::Command,
    page: usize,
  },
  Observe(bool),
  #[cfg(feature = "bench-events")]
  Snapshot,
  #[cfg(feature = "bench-events")]
  Mark {
    name: String,
    trace: Arc<crate::trace::Log>,
    ack: Option<mpsc::Sender<std::result::Result<(), String>>>,
  },
  Resize(Screen),
  SystemPalette(vpdf_model::theme::Palette),
  ReduceMotion(bool),
  Key(KeyEvent),
  Paste(String),
  Mouse {
    event: MouseEvent,
    pixels: (u16, u16),
    screen: Screen,
  },
  /// A trackpad turn or pinch; `event` gives its wheel direction.
  Gesture {
    event: MouseEvent,
    gesture: Gesture,
    pixels: (u16, u16),
    screen: Screen,
  },
  History {
    forward: bool,
    pixels: (u16, u16),
    screen: Screen,
  },
  Leave,
  Focus(bool),
  Fullscreen(bool),
  Open(PathBuf),
  Reference(vpdf_model::reference::Reference),
  Presented {
    content: bool,
    bytes: usize,
  },
  Wake,
  Close,
}

pub struct Image {
  pub id: TileId,
  pub pixels: Arc<Vec<u8>>,
}

#[derive(
  Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, serde::Serialize,
)]
#[serde(transparent)]
pub struct Ticket(u64);

pub struct Metrics {
  pub ticket: Ticket,
  pub built: Instant,
  pub build: Duration,
  pub busy: bool,
  pub animating: bool,
  pub searching: bool,
  pub sections: usize,
  pub page: Option<vpdf_model::text::PageId>,
  pub observation: Observation,
  pub observation_time: Option<Duration>,
  pub scroll: f32,
}

impl Default for Metrics {
  fn default() -> Self {
    Self {
      ticket: Ticket::default(),
      built: Instant::now(),
      build: Duration::ZERO,
      busy: false,
      animating: false,
      searching: false,
      sections: 0,
      page: None,
      observation: Observation::default(),
      observation_time: None,
      scroll: 0.0,
    }
  }
}

#[derive(Default, serde::Serialize)]
pub struct Observation {
  pub document: Option<PathBuf>,
  pub pane: Option<u32>,
  pub tab: usize,
  pub pane_count: usize,
  pub overview: bool,
  pub panel: &'static str,
  pub document_generation: u64,
  pub query: String,
  pub query_generation: u64,
  pub selected_hit: Option<usize>,
  pub search_outcome: &'static str,
  pub error: Option<String>,
  pub resources: Vec<Resources>,
  pub pages: usize,
  pub hits: usize,
  pub animating: bool,
}

impl Observation {
  pub(crate) fn pane(pane: &Pane) -> Self {
    pane.document().map_or_else(
      || Self { pane: Some(pane.id), panel: "RECENT", ..Self::default() },
      Self::of,
    )
  }

  pub(crate) fn of(app: &crate::app::App) -> Self {
    let position = app.reader.cursor;
    let hits = app.search.hits();
    let selected_hit = hits
      .partition_point(|hit| hit.first <= position)
      .checked_sub(1)
      .filter(|&index| position <= hits[index].last)
      .map(|index| index + 1);
    Self {
      document: Some(app.document_identity().to_owned()),
      pane: Some(app.id),
      tab: 1,
      pane_count: 1,
      overview: app.overlays.overview().is_some(),
      panel: app.panel.as_ref().map_or("none", |p| p.view.kind().title()),
      document_generation: app.document_generation,
      query: app.search.query().text.clone(),
      query_generation: app.search.job_generation(),
      selected_hit,
      search_outcome: app.search.outcome(),
      error: app.error.clone(),
      resources: Vec::new(),
      pages: app.doc.pages.len(),
      hits: hits.len(),
      animating: app.reader.animation().active(),
    }
  }
}

#[derive(serde::Serialize)]
pub struct Resources {
  pub owner: u32,
  pub visible: bool,
  pub text_bytes: usize,
  pub tile_bytes: usize,
  pub active_workers: usize,
  pub search_workers: usize,
}

impl Resources {
  pub(crate) fn of(
    app: &crate::app::App,
    visible: bool,
    tile_bytes: usize,
  ) -> Self {
    Self {
      owner: app.id,
      visible,
      text_bytes: app.doc.cache_bytes(),
      tile_bytes,
      active_workers: app.active_workers(),
      search_workers: usize::from(app.search_worker.is_some()),
    }
  }
}

pub struct Frame {
  pub ui: Arc<commands::Snapshot>,
  pub effects: Vec<commands::Effect>,
  pub metrics: Metrics,
  pub ui_font_size: f32,
  pub screen: Screen,
  pub scene: Scene<'static>,
  pub images: Vec<Image>,
  pub owners: Vec<TileOwner>,
  /// Owners whose document reloaded; their kept images are stale.
  pub reloaded: Vec<TileOwner>,
  pub title: String,
  pub cursor: vpdf_model::style::Shape,
  pub complete: bool,
  pub content: bool,
  pub quit: bool,
  pub clipboard: Option<String>,
}

impl Frame {
  pub fn wheel_step(&self) -> [f32; 2] {
    let rows = f32::from(vpdf_ui::pointer::WHEEL_ROWS);
    [
      f32::from(self.screen.cell_width) * rows,
      f32::from(self.screen.cell_height) * rows,
    ]
  }
}

pub struct Bridge {
  sender: Sender,
  latest: Arc<Mutex<Option<Frame>>>,
  thread: Option<JoinHandle<Result<()>>>,
  fullscreen: bool,
}

#[derive(Default)]
pub struct Launch {
  pub config: crate::config::Loaded,
  pub references: Vec<vpdf_model::reference::Reference>,
  /// Reading state; the window session is saved beside it.
  pub store: Option<PathBuf>,
  /// The system asks apps to reopen their windows.
  pub system_reopen: bool,
}

impl Launch {
  fn reopen(&self) -> bool {
    match self.config.settings.restore_session() {
      crate::config::Restore::System => self.system_reopen,
      crate::config::Restore::Always => true,
      crate::config::Restore::Never => false,
    }
  }
}

/// Queues worker input from any thread.
#[derive(Clone)]
pub struct Sender {
  send: mpsc::Sender<(Ticket, Input)>,
  next: Arc<AtomicU64>,
}

impl Sender {
  pub fn send(&self, input: Input) -> Ticket {
    let ticket = Ticket(self.next.fetch_add(1, Ordering::Relaxed));
    let _ = self.send.send((ticket, input));
    ticket
  }
}

impl Bridge {
  pub fn configured(
    launch: Launch,
    repaint: impl Fn() + Send + Sync + 'static,
  ) -> Result<Self> {
    let mut windows = launch.store.as_deref().and_then(restore::Store::claim);
    let layout = windows
      .as_mut()
      .filter(|_| launch.reopen())
      .and_then(restore::Store::load);
    let fullscreen = layout.as_ref().is_some_and(|layout| layout.fullscreen);
    let (send, receive) = mpsc::channel();
    let latest = Arc::new(Mutex::new(None));
    let output = Arc::clone(&latest);
    let wake_send = send.clone();
    let pending = Arc::new(AtomicBool::new(false));
    let signaled = Arc::clone(&pending);
    let wake = Wake::new(move || {
      if !signaled.swap(true, Ordering::AcqRel) {
        let _ = wake_send.send((Ticket::default(), Input::Wake));
      }
    });
    let worker =
      thread::Builder::new().name("vpdf-session".into()).spawn(move || {
        run(
          launch,
          Resume { store: windows, layout },
          &receive,
          &output,
          &repaint,
          &wake,
          &pending,
        )
      })?;
    Ok(Self {
      sender: Sender { send, next: Arc::new(AtomicU64::new(1)) },
      latest,
      thread: Some(worker),
      fullscreen,
    })
  }

  /// The restored window filled its screen.
  pub fn fullscreen(&self) -> bool {
    self.fullscreen
  }

  pub fn send(&self, input: Input) -> Ticket {
    self.sender.send(input)
  }

  pub fn sender(&self) -> Sender {
    self.sender.clone()
  }

  pub fn take(&self) -> Option<Frame> {
    self.latest.lock().ok()?.take()
  }

  pub fn close(&mut self) -> Result<()> {
    self.send(Input::Close);
    if let Some(worker) = self.thread.take() {
      worker.join().map_err(|_| anyhow::anyhow!("Session thread failed"))??;
    }
    Ok(())
  }
}

impl Drop for Bridge {
  fn drop(&mut self) {
    let _ = self.close();
  }
}

/// The claimed window session and the layout to reopen.
struct Resume {
  store: Option<restore::Store>,
  layout: Option<restore::Layout>,
}

impl Resume {
  fn poll(&mut self, session: &mut Session, now: Instant) -> Option<Duration> {
    self.store.as_mut()?.poll(session, now)
  }

  fn touch(&mut self, now: Instant) {
    if let Some(store) = &mut self.store {
      store.touch(now);
    }
  }

  fn flush(&mut self, session: &mut Session) {
    if let Some(store) = &mut self.store {
      store.flush(session);
    }
  }
}

fn run(
  launch: Launch,
  mut windows: Resume,
  receive: &mpsc::Receiver<(Ticket, Input)>,
  latest: &Mutex<Option<Frame>>,
  repaint: &impl Fn(),
  wake: &Wake,
  pending: &AtomicBool,
) -> Result<()> {
  let mut starting = !launch.references.is_empty();
  let mut session = Session::configured(launch.store, launch.config);
  session.frontend = crate::raster::Frontend::Native(wake.clone());
  session.windows = restore::Windows::new(windows.layout.take());
  let mut screen = Screen {
    columns: 100,
    rows: 40,
    cell_width: 10,
    cell_height: 20,
    origin: (0, 0),
  };
  ensure_home(&mut session, screen);
  for reference in launch.references {
    apply(&mut session, Input::Reference(reference), &mut screen);
  }
  session.resize(screen)?;
  let mut cache = Cache::new(BUDGET);
  let mut reloaded = Vec::new();
  let mut chores = Chores::new(Instant::now());
  let mut dirty = true;
  let mut animation_due = Instant::now();
  let mut applied = Ticket::default();
  loop {
    dirty |= commands::copy_update(&mut session);
    dirty |= session.clean_state();
    dirty |= session.help_updates(&mut std::io::sink());
    dirty |= session.print_update();
    dirty |= picker_update(&mut session, screen);
    dirty |= resume_keys(&mut session);
    dirty |= open_next(&mut session, &mut screen);
    finish_launch(&mut session, &mut starting)?;
    quit_when_closed(&mut session);
    session.perf.running(session.state.perf);
    session.perf.view.output("Upload");
    let now = Instant::now();
    let sampled = session.state.perf && session.perf.wait(now).is_zero();
    if sampled {
      session.perf.sample(now);
    }
    dirty |= session.recent_updates();
    if !session.tabs.is_empty() {
      dirty |= session.maintain(now);
      dirty |= completions(&mut session, &mut cache, &mut reloaded);
      dirty |= chores.run(&mut session, now);
      dirty |= animate(&mut session, &mut animation_due, now);
    }
    if dirty || sampled {
      let frame = frame(&mut session, &mut cache, screen);
      match frame {
        Ok(mut frame) => {
          frame.metrics.ticket = applied;
          frame.content = dirty;
          frame.reloaded = std::mem::take(&mut reloaded);
          publish(latest, frame);
        }
        Err(error) => {
          eprintln!("Native frame: {error}");
        }
      }
      repaint();
      dirty = !session.tabs.is_empty() && session.redraw_due();
      if dirty && !session.quit {
        continue;
      }
    }
    if session.quit {
      break;
    }
    let now = Instant::now();
    let mut wait = wait_time(&mut session, &chores, animation_due, now);
    if let Some(due) = windows.poll(&mut session, now) {
      wait = wait.min(due);
    }
    let input = match receive.recv_timeout(wait) {
      Ok(input) => input,
      Err(mpsc::RecvTimeoutError::Timeout) => continue,
      Err(mpsc::RecvTimeoutError::Disconnected) => break,
    };
    // Queued input lands in one frame rather than one frame each.
    let inputs = std::iter::once(input)
      .chain(receive.try_iter().take(BATCH))
      .map(|(ticket, input)| {
        applied = applied.max(ticket);
        input
      });
    let drained =
      drain(&mut session, inputs, &mut chores, &mut screen, pending);
    windows.touch(Instant::now());
    match drained {
      Some(applied) => dirty |= applied,
      None => break,
    }
  }
  windows.flush(&mut session);
  session.finish()
}

/// Opens one queued file, else one saved tab; whether either ran.
fn open_next(session: &mut Session, screen: &mut Screen) -> bool {
  if session.blocked() || !session.pending_keys.is_empty() {
    return false;
  }
  if let Some(path) = session.native_opens.pop_front() {
    apply(session, Input::Reference(path), screen);
    return true;
  }
  session.restore_next(*screen)
}

/// Applies queued input; whether any needs a frame, or None on close.
fn drain(
  session: &mut Session,
  inputs: impl Iterator<Item = Input>,
  chores: &mut Chores,
  screen: &mut Screen,
  pending: &AtomicBool,
) -> Option<bool> {
  let mut applied = false;
  for input in inputs {
    match input {
      #[cfg(feature = "bench-events")]
      Input::Mark { name, trace, ack } => {
        let result =
          trace.mark(Instant::now(), &name).map_err(|error| error.to_string());
        if let Err(error) = &result {
          eprintln!("Trace mark failed: {error}");
        }
        if let Some(ack) = ack {
          let _ = ack.send(result);
        }
        applied = true;
      }
      #[cfg(feature = "bench-events")]
      Input::Snapshot => applied = true,
      Input::Observe(enabled) => {
        session.observing = enabled.then_some(());
        applied = true;
      }
      Input::Close => return None,
      Input::Wake => pending.store(false, Ordering::Release),
      Input::Presented { content, bytes } => {
        let now = Instant::now();
        if content {
          session.perf.frame(now);
        }
        session.drawn();
        session.perf.bytes(now, bytes);
      }
      input => {
        chores.touch(Instant::now());
        applied |= apply(session, input, screen);
        // Queued keys and opens resume first, at the top of the loop.
        if session.quit
          || session.picker.is_some()
          || !session.pending_keys.is_empty()
          || !session.native_opens.is_empty()
        {
          break;
        }
      }
    }
  }
  Some(applied)
}

fn finish_launch(session: &mut Session, starting: &mut bool) -> Result<()> {
  if !*starting || session.unlock.is_some() || !session.native_opens.is_empty()
  {
    return Ok(());
  }
  if !session.tabs.is_empty() {
    session.activate(0)?;
  }
  *starting = false;
  Ok(())
}

fn resume_keys(session: &mut Session) -> bool {
  if session.blocked() {
    return false;
  }
  if session.quit || session.tabs.is_empty() {
    session.pending_keys.clear();
    return false;
  }
  let keys = std::mem::take(&mut session.pending_keys);
  if keys.is_empty() {
    return false;
  }
  let result = session.replay_keys(keys.into());
  if result.is_err() || session.quit || session.tabs.is_empty() {
    session.pending_keys.clear();
  }
  report(session, result);
  true
}

fn picker_update(session: &mut Session, screen: Screen) -> bool {
  use std::task::Poll;
  let Some(picker) = &mut session.picker else { return false };
  let tab = picker.tab;
  let result = picker.poll();
  if matches!(result, Ok(Poll::Pending)) {
    return false;
  }
  session.picker = None;
  let result = result.and_then(|picked| {
    let Poll::Ready(Some(path)) = picked else { return Ok(()) };
    picked_path(session, path, tab, screen)
  });
  if result.is_err() {
    session.pending_keys.clear();
  }
  report(session, result);
  true
}

fn picked_path(
  session: &mut Session,
  path: PathBuf,
  tab: bool,
  screen: Screen,
) -> Result<()> {
  let operation = if session.tabs.is_empty() {
    Operation::Open(path)
  } else {
    Operation::Request(super::Request::Open { path, tab })
  };
  session.resume(operation, screen)
}

fn set_motion(session: &mut Session, reduce: bool) -> bool {
  let motion = if reduce { Motion::Reduced } else { Motion::Full };
  let changed = session.motion != motion;
  session.motion = motion;
  changed
}

fn animate(session: &mut Session, due: &mut Instant, now: Instant) -> bool {
  if session.motion == Motion::Reduced {
    *due = now;
    let mut changed = false;
    for tab in &mut session.tabs {
      for app in tab.panes.iter_mut().filter_map(Pane::document_mut) {
        changed |= app.reader.settle_scroll();
      }
    }
    return changed;
  }
  if !session.animating() {
    *due = now;
    return false;
  }
  if now < *due {
    return false;
  }
  *due = crate::pace::next(*due, now);
  session.animate(now)
}

fn wait_time(
  session: &mut Session,
  chores: &Chores,
  animation_due: Instant,
  now: Instant,
) -> Duration {
  let restoring =
    session.windows.restoring.as_ref().is_some_and(restore::Restoring::waiting);
  if !session.blocked()
    && (!session.native_opens.is_empty()
      || !session.pending_keys.is_empty()
      || restoring)
  {
    return Duration::ZERO;
  }
  if commands::copying(session) {
    return Duration::ZERO;
  }
  let mut wait = if session.tabs.is_empty() {
    Duration::from_secs(2)
  } else {
    let wait = chores.wait(session, now, false);
    if session.animating() {
      wait.min(animation_due.saturating_duration_since(now))
    } else {
      wait
    }
  };
  if session.state.perf {
    wait = wait.min(session.perf.wait(now));
  }
  if session.picker.is_some() {
    wait = wait.min(Duration::from_millis(50));
  }
  session.print_wait(wait)
}

fn publish(latest: &Mutex<Option<Frame>>, mut frame: Frame) {
  let Ok(mut output) = latest.lock() else { return };
  if frame.clipboard.is_none() {
    frame.clipboard =
      output.as_mut().and_then(|previous| previous.clipboard.take());
  }
  frame.content |= output.as_ref().is_some_and(|previous| previous.content);
  if let Some(previous) = output.as_mut() {
    frame.reloaded.append(&mut previous.reloaded);
    let mut effects = std::mem::take(&mut previous.effects);
    effects.append(&mut frame.effects);
    frame.effects = effects;
  }
  *output = Some(frame);
}

pub(super) fn apply(
  session: &mut Session,
  input: Input,
  screen: &mut Screen,
) -> bool {
  commands::refresh(session);
  let changed = match input {
    Input::Action(command) => commands::dispatch(session, command, *screen),
    Input::PasteTarget { command, text } => {
      commands::accepts(session, command, commands::Action::Paste)
        && apply_reader(session, Input::Paste(text), screen)
    }
    Input::Page { command, page } => {
      commands::go_to_page(session, command, page)
    }
    Input::Picked { request, paths } => {
      commands::picked(session, request, paths)
    }
    input => {
      if matches!(
        input,
        Input::Key(_)
          | Input::Mouse {
            event: MouseEvent {
              kind: vpdf_model::input::MouseEventKind::Down(_)
                | vpdf_model::input::MouseEventKind::Drag(_),
              ..
            },
            ..
          }
          | Input::History { .. }
      ) {
        commands::cancel_selection(session);
      }
      apply_reader(session, input, screen)
    }
  };
  commands::refresh(session);
  changed
}

fn apply_reader(
  session: &mut Session,
  input: Input,
  screen: &mut Screen,
) -> bool {
  if session.picker.is_some()
    && matches!(
      input,
      Input::Key(_)
        | Input::Paste(_)
        | Input::Mouse { .. }
        | Input::Gesture { .. }
        | Input::History { .. }
    )
  {
    return false;
  }
  commands::prepare_prompt(session);
  match session.password_input(&input, *screen) {
    Ok(true) => {
      return !matches!(
        input,
        Input::Mouse { .. } | Input::Gesture { .. } | Input::History { .. }
      );
    }
    Err(error) => {
      report(session, Err(error));
      return true;
    }
    Ok(false) => {}
  }
  let applied = match input {
    Input::SystemPalette(palette) => Ok(session.set_system_palette(palette)),
    Input::ReduceMotion(reduce) => Ok(set_motion(session, reduce)),
    Input::Resize(size) => resize(session, screen, size),
    Input::Open(path) => {
      return apply(session, Input::Reference(path.into()), screen);
    }
    Input::Reference(_) if session.confirming_removal() => Ok(false),
    Input::Reference(reference) => open_input(session, reference, *screen),
    Input::Key(key) => session.interact(Interaction::Key(key)),
    Input::Paste(text) => session.interact(Interaction::Paste(text)),
    Input::Mouse { event, pixels, screen: drawn } if drawn == *screen => {
      mouse_input(session, event, pixels, *screen)
    }
    Input::Gesture { event, gesture, pixels, screen: drawn }
      if drawn == *screen =>
    {
      gesture_input(session, pixel_report(event, pixels), gesture, *screen)
    }
    Input::History { forward, pixels: (x, y), screen: drawn }
      if drawn == *screen && !session.tabs.is_empty() =>
    {
      use vpdf_model::input::{Button, Mouse, MouseKind, Spot};
      let button = if forward { Button::FORWARD } else { Button::BACK };
      let report = Mouse {
        kind: MouseKind::Press(button),
        at: Spot::Pixel { x, y },
        modifiers: vpdf_model::input::KeyModifiers::NONE,
      };
      session.interact(Interaction::Mouse(report))
    }
    Input::Leave => session.interact(Interaction::Leave).inspect(|_changed| {
      session.drop_pointer();
    }),
    Input::Fullscreen(full) => Ok(session.windows.fullscreen(full)),
    Input::Focus(focused) => {
      session.sight =
        if focused { super::Sight::Focused } else { super::Sight::Unfocused };
      if !focused {
        session.pointer_left();
        session.drop_pointer();
      }
      Ok(true)
    }
    _ => Ok(false),
  };
  quit_when_closed(session);
  applied.unwrap_or_else(|error| {
    report(session, Err(error));
    true
  })
}

fn pixel_report(
  event: MouseEvent,
  (x, y): (u16, u16),
) -> vpdf_model::input::Mouse {
  let mut report = vpdf_model::input::Mouse::from(event);
  report.at = vpdf_model::input::Spot::Pixel { x, y };
  report
}

fn mouse_input(
  session: &mut Session,
  event: MouseEvent,
  pixels: (u16, u16),
  screen: Screen,
) -> Result<bool> {
  let report = pixel_report(event, pixels);
  let _ = screen;
  session.interact(Interaction::Mouse(report))
}

fn gesture_input(
  session: &mut Session,
  report: vpdf_model::input::Mouse,
  gesture: Gesture,
  _screen: Screen,
) -> Result<bool> {
  session.interact(Interaction::Gesture(report, gesture))
}

fn resize(
  session: &mut Session,
  screen: &mut Screen,
  size: Screen,
) -> Result<bool> {
  *screen = size;
  session.windows.sized();
  session.resize(size).map(|()| true)
}

fn open_input(
  session: &mut Session,
  reference: vpdf_model::reference::Reference,
  screen: Screen,
) -> Result<bool> {
  if session.blocked() {
    session.native_opens.push_back(reference);
    Ok(true)
  } else {
    session.resume(Operation::Reference(reference), screen).map(|()| true)
  }
}

// Like the terminal, closing the last tab exits.
fn quit_when_closed(session: &mut Session) {
  if session.tabs.is_empty()
    && session.unlock.is_none()
    && session.windows.restoring.is_none()
  {
    session.quit = true;
  }
}

fn ensure_home(session: &mut Session, screen: Screen) {
  if session.tabs.is_empty() && !session.quit {
    let result = session.recent_tab(screen);
    report(session, result);
  }
}

fn report(session: &mut Session, result: Result<()>) {
  if let Err(error) = result
    && !session.tabs.is_empty()
  {
    session.focused_mut().fail(error.to_string());
  }
}

fn completions(
  session: &mut Session,
  cache: &mut Cache<TileId, Arc<Vec<u8>>>,
  reloaded: &mut Vec<TileOwner>,
) -> bool {
  let mut dirty = session.image_updates(&mut std::io::sink());
  let mut evicted = Vec::new();
  for app in session.tab().panes.iter_mut().filter_map(Pane::document_mut) {
    let stale = std::mem::take(&mut app.images_stale);
    if stale {
      let owner = TileOwner::new(app.id);
      let old: Vec<_> = cache
        .entries()
        .filter(|(id, _)| id.owner == owner)
        .map(|(id, _)| *id)
        .collect();
      for id in old {
        cache.remove(&id);
      }
      reloaded.push(owner);
      app.tiles.invalidate();
      dirty = true;
    }
    let Some(raw) = &app.worker.raw else { continue };
    let overflow = raw.overflow.swap(false, Ordering::AcqRel);
    for result in raw.results.try_iter() {
      match result {
        Ok(tile) => {
          if tile.key.recolor != app.layout.recolor() {
            continue;
          }
          dirty |= app.tiles.wants(&tile.key);
          let weight = tile.pixels.len();
          let id = TileId { owner: TileOwner::new(app.id), key: tile.key };
          evicted.append(&mut cache.insert(id, tile.pixels.into(), weight));
        }
        Err(error) => {
          app.error = Some(error);
          dirty = true;
        }
      }
    }
    if overflow {
      app.track_jobs();
      app.worker = crate::raster::Raster::raw(
        &app.path,
        &app.password,
        app.doc.lists(),
        &app.wake,
        false,
      );
      app.tiles.invalidate();
      dirty = true;
    }
    dirty |= app.job_updates();
  }
  for app in session.panes_mut() {
    if evicted
      .iter()
      .any(|id| id.owner.get() == app.id && app.tiles.wants(&id.key))
    {
      app.tiles.invalidate();
      dirty = true;
    }
  }
  dirty
}

fn prepare_scene(
  session: &mut Session,
  scene: &mut impl vpdf_model::scene::Draw<'static>,
) {
  session.settings.scene_theme(
    scene,
    vpdf_model::theme::Colors::Palette(session.system_palette),
  );
  session.present_startup();
  commands::prepare_prompt(session);
}

fn resolve_themes(scene: &mut [Op<'_>], palette: vpdf_model::theme::Palette) {
  for op in scene {
    if let Op::Theme(theme) = op
      && theme.colors == vpdf_model::theme::Colors::Terminal
    {
      theme.colors = vpdf_model::theme::Colors::Palette(palette);
    }
  }
}

fn frame(
  session: &mut Session,
  cache: &mut Cache<TileId, Arc<Vec<u8>>>,
  screen: Screen,
) -> Result<Frame> {
  let started = Instant::now();
  let owners: Vec<_> = session
    .tabs
    .get(session.active)
    .into_iter()
    .flat_map(|tab| &tab.panes)
    .filter_map(Pane::document)
    .map(|app| TileOwner::new(app.id))
    .collect();
  let visible: HashSet<_> = owners.iter().map(|owner| owner.get()).collect();
  let retired: Vec<_> = cache
    .entries()
    .filter(|(id, _)| !visible.contains(&id.owner.get()))
    .map(|(id, _)| *id)
    .collect();
  for id in retired {
    cache.remove(&id);
  }
  let mut recorder = Recorder::new(|id| cache.get(&id).is_some());
  let mut complete = true;
  prepare_scene(session, &mut recorder);
  let title = if session.tabs.is_empty() {
    "Vpdf".into()
  } else {
    session.prepare_frame();
    let area = session.area();
    let tab = &mut session.tabs[session.active];
    Session::scene_tab(
      tab,
      area,
      session.unmeasured.as_ref(),
      &mut recorder,
      |pane, scene| {
        pane.scene_pane(scene, false)?;
        complete &= pane.document().is_none_or(|app| app.complete);
        Ok(())
      },
    )?;
    let title = tab.panes[tab.focus].title();
    session.finish_frame();
    title
  };
  if session.state.perf {
    session.perf.view.scene(&mut recorder, screen, true);
  }
  if let Some(unlock) = &session.unlock {
    if unlock.retry && screen.rows > 1 {
      super::Notice::new(
        screen.rows - 1,
        screen.columns,
        "Wrong password".into(),
      )
      .scene(&mut recorder);
    }
    unlock.prompt.scene(&mut recorder, screen, super::At::default(), None);
  }
  if screen.rows == 0 {
    complete = false;
  }
  let mut scene = recorder.into_scene();
  resolve_themes(&mut scene, session.system_palette);
  let mut seen = HashSet::new();
  let images = scene
    .iter()
    .filter_map(|op| {
      let Op::Tile(placement) = op else { return None };
      let id = placement.id();
      if !seen.insert(id) {
        return None;
      }
      Some(Image { id, pixels: Arc::clone(cache.get(&id)?) })
    })
    .collect();
  let clipboard = take_clipboard(session);
  let cursor = if session.tabs.is_empty() || !vpdf_terminal::terminal::mouse() {
    vpdf_model::style::Shape::Default
  } else {
    session.shape()
  };
  commands::refresh(session);
  Ok(Frame {
    ui: Arc::clone(&session.native.snapshot),
    effects: std::mem::take(&mut session.native.effects),
    metrics: frame_metrics(session, started, cache, &visible),
    ui_font_size: session.settings.ui_font_size(),
    screen,
    scene,
    images,
    owners,
    reloaded: Vec::new(),
    title,
    cursor,
    complete,
    content: true,
    quit: session.quit,
    clipboard,
  })
}

fn take_clipboard(session: &mut Session) -> Option<String> {
  let pane = session
    .panes_mut()
    .filter_map(|app| app.reader.take_yank())
    .filter(|yank| crate::clipboard::synced(yank.register))
    .fold(None, |_, yank| Some(yank.text));
  session.native.clipboard.take().or(pane)
}

fn frame_metrics(
  session: &Session,
  started: Instant,
  cache: &Cache<TileId, Arc<Vec<u8>>>,
  visible: &HashSet<u32>,
) -> Metrics {
  let mut metrics = metrics(session, started);
  if session.observing.is_some() {
    let observing = Instant::now();
    metrics.observation = session
      .tabs
      .get(session.active)
      .map_or_else(Observation::default, |tab| {
        Observation::pane(&tab.panes[tab.focus])
      });
    metrics.observation.animating = metrics.animating;
    metrics.observation.tab = session.active + 1;
    metrics.observation.pane_count =
      session.tabs.get(session.active).map_or(0, |tab| tab.panes.len());
    metrics.observation.resources = session
      .panes()
      .map(|app| {
        let tiles = cache.entries().filter(|(id, _)| id.owner.get() == app.id);
        let bytes = tiles.map(|(_, pixels)| pixels.len()).sum();
        Resources::of(app, visible.contains(&app.id), bytes)
      })
      .collect();
    metrics.observation_time = Some(observing.elapsed());
  }
  metrics.built = Instant::now();
  metrics.build = metrics.built.saturating_duration_since(started);
  metrics
}

fn metrics(session: &Session, started: Instant) -> Metrics {
  let tab = session.tabs.get(session.active);
  let apps =
    || tab.into_iter().flat_map(|tab| &tab.panes).filter_map(Pane::document);
  let animating = apps().any(|app| app.reader.animation().active());
  let busy = apps().any(crate::app::App::busy);
  let focused = tab.and_then(|tab| tab.panes[tab.focus].document());
  let built = Instant::now();
  Metrics {
    ticket: Ticket::default(),
    built,
    build: built.saturating_duration_since(started),
    busy,
    animating,
    searching: focused.is_some_and(|app| app.search.phase().running()),
    sections: focused.map_or(0, |app| app.doc.outline.len()),
    page: focused.map(|app| app.navigation_position().page),
    observation: Observation::default(),
    observation_time: None,
    scroll: focused.map_or(0.0, |app| app.reader.scroll),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::pace::FRAME as ANIMATION;
  use crate::screen::cells;
  use vpdf_model::input::{KeyCode, KeyModifiers, MouseEventKind};

  pub(super) fn wait(
    bridge: &Bridge,
    wakes: &mpsc::Receiver<()>,
    accept: impl Fn(&Frame) -> bool,
  ) -> Result<Frame> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
      if let Some(frame) = bridge.take()
        && accept(&frame)
      {
        return Ok(frame);
      }
      wakes.recv_timeout(deadline.saturating_duration_since(Instant::now()))?;
    }
  }

  fn launch(
    paths: Vec<PathBuf>,
    store: Option<PathBuf>,
    page: Option<usize>,
  ) -> Result<(Bridge, mpsc::Receiver<()>)> {
    let (wake, wakes) = crate::fixtures::wake();
    let config = crate::config::Loaded::default();
    Ok((
      Bridge::configured(
        Launch {
          references: {
            let mut references: Vec<vpdf_model::reference::Reference> =
              paths.into_iter().map(Into::into).collect();
            if let Some(page) = page {
              references[0].operations.push(
                vpdf_model::reference::Operation::Page(
                  std::num::NonZeroUsize::new(page)
                    .expect("Test page is positive"),
                ),
              );
            }
            references
          },
          store,
          config,
          ..Launch::default()
        },
        move || wake.wake(),
      )?,
      wakes,
    ))
  }

  fn masked(frame: &Frame) -> bool {
    frame
      .scene
      .iter()
      .any(|op| matches!(op, Op::InputArea { masked: true, .. }))
  }

  fn shows(scene: &Scene, needle: &str) -> bool {
    scene
      .iter()
      .any(|op| matches!(op, Op::Text(text) if text.as_str().contains(needle)))
  }

  fn sized() -> Screen {
    crate::screen::pixels(100, 40, 10, 20)
  }

  fn native() -> Session {
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
    session
  }

  fn temp_pdf(pages: &[&str]) -> Result<(tempfile::TempDir, PathBuf)> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("source.pdf");
    crate::fixtures::pdf(&path, pages, "")?;
    Ok((root, path))
  }

  fn opened(
    pages: &[&str],
    screen: Screen,
  ) -> Result<(tempfile::TempDir, PathBuf, Session)> {
    let (root, path) = temp_pdf(pages)?;
    let mut session = native();
    session.open(&path, true, screen)?;
    session.native.effects.clear();
    Ok((root, path, session))
  }

  fn mouse(kind: MouseEventKind, pixels: (u16, u16), screen: Screen) -> Input {
    let (column, row) =
      (pixels.0 / screen.cell_width, pixels.1 / screen.cell_height);
    Input::Mouse { event: MouseEvent::new(kind, column, row), pixels, screen }
  }

  #[test]
  fn prompts_inset_first_frame() -> Result<()> {
    let mut screen = sized();
    let (_root, _path, mut session) = opened(&["body"], screen)?;
    let mut cache = Cache::new(BUDGET);
    for ch in [':', '/', '?'] {
      apply(&mut session, key(KeyCode::Char(ch)), &mut screen);
      let rendered = frame(&mut session, &mut cache, screen)?;
      assert!(rendered.scene.ends_with(&[
        Op::Move { row: 39, column: 3 },
        Op::InputArea { column: 2, columns: 98, masked: false },
        Op::BlockCursor,
        Op::ShowCursor,
      ]));
      apply(&mut session, key(KeyCode::Esc), &mut screen);
    }
    let mut session = native();
    ensure_home(&mut session, screen);
    apply(&mut session, key(KeyCode::Char(':')), &mut screen);
    let rendered = frame(&mut session, &mut cache, screen)?;
    assert!(rendered.scene.contains(&Op::InputArea {
      column: 2,
      columns: 98,
      masked: false,
    }));
    Ok(())
  }

  #[test]
  fn reduced_motion_settles_navigation() -> Result<()> {
    let mut screen = sized();
    let (_root, _path, mut session) = opened(&["navigation"], screen)?;
    let now = Instant::now();
    let mut due = now + Duration::from_secs(1);
    session.app().unwrap().reader.animate_to(100.0, now);
    assert!(apply(&mut session, Input::ReduceMotion(true), &mut screen));
    assert!(!apply(&mut session, Input::ReduceMotion(true), &mut screen));
    assert!(animate(&mut session, &mut due, now));
    assert!(!session.app().unwrap().reader.animation().active());
    assert_eq!(session.app().unwrap().reader.scroll, 100.0);
    assert!(!animate(&mut session, &mut due, now));
    assert!(apply(&mut session, Input::ReduceMotion(false), &mut screen));
    session.app().unwrap().reader.animate_to(200.0, now);
    animate(&mut session, &mut due, now);
    assert!(session.app().unwrap().reader.animation().active());
    Ok(())
  }

  #[test]
  fn font_reload_updates_frame() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let config = super::super::tests::settings_file(directory.path());
    let mut session = Session::configured(None, config);
    let screen = sized();
    ensure_home(&mut session, screen);
    let mut cache = Cache::new(BUDGET);
    assert_eq!(frame(&mut session, &mut cache, screen)?.ui_font_size, 16.0);
    std::fs::write(
      directory.path().join("config.toml"),
      "[appearance]\nui_font_size = 24",
    )?;
    session.reload_config();
    assert_eq!(frame(&mut session, &mut cache, screen)?.ui_font_size, 24.0);
    Ok(())
  }

  #[test]
  fn native_system_repaints() -> Result<()> {
    use vpdf_model::theme::{Colors, Palette, Theme};
    let mut session = native();
    let mut screen = sized();
    ensure_home(&mut session, screen);
    let directory = tempfile::tempdir()?;
    let config = directory.path().join("opacity.toml");
    std::fs::write(&config, "[appearance.opacity]\nselection = 0.6")?;
    session.settings =
      crate::config::Options { config: Some(config) }.load().settings;
    let opacity = session.settings.opacity();
    for palette in [Palette::light(), Palette::dark()] {
      assert!(apply(&mut session, Input::SystemPalette(palette), &mut screen));
      assert!(!apply(&mut session, Input::SystemPalette(palette), &mut screen));
      let rendered = frame(&mut session, &mut Cache::new(BUDGET), screen)?;
      assert_eq!(
        rendered.scene.first(),
        Some(&Op::theme(Colors::Palette(palette), opacity))
      );
      assert!(rendered.scene.iter().all(|op| match op {
        Op::Theme(theme) =>
          theme.colors == Colors::Palette(palette) && theme.opacity == opacity,
        _ => true,
      }));
      assert_eq!(
        session.settings.theme(),
        &crate::config::ThemeChoice::Builtin(Theme::System)
      );
    }
    session.set_theme(Theme::Light);
    assert!(!apply(
      &mut session,
      Input::SystemPalette(Palette::light()),
      &mut screen
    ));
    let rendered = frame(&mut session, &mut Cache::new(BUDGET), screen)?;
    assert_eq!(
      rendered.scene.first(),
      Some(&Op::theme(Colors::Palette(Palette::light()), opacity))
    );
    Ok(())
  }

  #[test]
  fn configured_native_startup() -> Result<()> {
    let (directory, source) = temp_pdf(&["one", "two"])?;
    for document in [false, true] {
      let mut session = Session::configured(
        None,
        super::super::tests::settings_file(directory.path()),
      );
      session.frontend = crate::raster::Frontend::Native(Wake::none());
      let screen = sized();
      session.screen = Some(screen);
      ensure_home(&mut session, screen);
      if document {
        session.open(&source, true, screen)?;
        session.app().unwrap().reader.notice("Ordinary startup");
      }
      let warning = session.startup_warning.clone().unwrap();
      let rendered = frame(&mut session, &mut Cache::new(BUDGET), screen)?;
      assert!(shows(&rendered.scene, "Config"));
      if document {
        assert_eq!(session.app().unwrap().reader.message, warning);
        assert!(!session.app().unwrap().cursorline);
        assert_eq!(session.app().unwrap().layout.zoom(), 2.0);
      } else {
        assert_eq!(
          session.recent().unwrap().view.error(),
          Some(warning.as_str())
        );
      }
    }
    Ok(())
  }

  #[test]
  fn configured_encrypted_retry() -> Result<()> {
    let (directory, source) = temp_pdf(&["one", "two"])?;
    let encrypted = protected(&source)?;
    let (wake, wakes) = crate::fixtures::wake();
    let mut bridge = Bridge::configured(
      Launch {
        references: vec![vpdf_model::reference::Reference::parse(
          format!("{}#page=2", encrypted.path().display()).as_ref(),
        )?],
        store: None,
        config: super::super::tests::settings_file(directory.path()),
        ..Launch::default()
      },
      move || wake.wake(),
    )?;
    wait(&bridge, &wakes, masked)?;
    bridge.send(Input::Paste("test-reader".into()));
    bridge.send(key(KeyCode::Enter));
    let rendered = wait(&bridge, &wakes, |frame| {
      frame.complete
        && frame.metrics.page == Some(vpdf_model::text::PageId(1))
        && shows(&frame.scene, "Config")
    })?;
    assert_eq!(rendered.metrics.page, Some(vpdf_model::text::PageId(1)));
    bridge.close()
  }

  #[test]
  fn startup_warning_cancel() -> Result<()> {
    let (directory, source) = temp_pdf(&["one"])?;
    let encrypted = protected(&source)?;
    let mut session = Session::configured(
      None,
      super::super::tests::settings_file(directory.path()),
    );
    session.frontend = crate::raster::Frontend::Native(Wake::none());
    let mut screen = sized();
    session.screen = Some(screen);
    ensure_home(&mut session, screen);
    apply(&mut session, Input::Open(encrypted.path().into()), &mut screen);
    let mut cache = Cache::new(BUDGET);
    frame(&mut session, &mut cache, screen)?;
    assert!(session.startup_warning.is_some());
    apply(&mut session, key(KeyCode::Esc), &mut screen);
    assert!(shows(&frame(&mut session, &mut cache, screen)?.scene, "Config"));
    assert!(session.startup_warning.is_none());
    Ok(())
  }

  #[test]
  fn observations_require_a_consumer() -> Result<()> {
    let (_root, path) = temp_pdf(&["observed"])?;
    let mut session = Session::new(None);
    let mut screen = sized();
    apply(&mut session, Input::Open(path.clone()), &mut screen);
    let cache = Cache::new(BUDGET);
    let visible = HashSet::new();
    let started = Instant::now();
    let disabled = frame_metrics(&session, started, &cache, &visible);
    assert!(disabled.observation_time.is_none());
    assert!(disabled.observation.document.is_none());
    assert!(disabled.observation.resources.is_empty());
    session.observing = Some(());
    let enabled = frame_metrics(&session, started, &cache, &visible);
    assert!(enabled.observation_time.is_some());
    assert_eq!(enabled.observation.document, Some(path.canonicalize()?));
    assert_eq!(enabled.observation.resources.len(), 1);
    assert!(enabled.built >= disabled.built);
    assert!(enabled.build >= enabled.observation_time.unwrap());
    session.app().unwrap().command("vsplit")?;
    session.settle()?;
    let tab = &mut session.tabs[session.active];
    tab.panes[1 - tab.focus]
      .document_mut()
      .unwrap()
      .reader
      .animate_to(100.0, started);
    let split = frame_metrics(&session, started, &cache, &visible);
    assert!(split.animating && split.observation.animating);
    Ok(())
  }

  #[test]
  fn frames_identify_applied_inputs() -> Result<()> {
    let root = tempfile::tempdir()?;
    let (mut bridge, wakes) =
      launch(Vec::new(), Some(root.path().join("state")), None)?;
    let initial = wait(&bridge, &wakes, |frame| frame.complete)?;
    let mut screen = sized();
    screen.columns += 1;
    let ticket = bridge.send(Input::Resize(screen));
    let updated =
      wait(&bridge, &wakes, |frame| frame.metrics.ticket >= ticket)?;
    assert!(ticket > initial.metrics.ticket);
    assert_eq!(updated.screen, screen);
    assert!(updated.metrics.built >= initial.metrics.built);
    bridge.close()
  }

  #[test]
  #[cfg(feature = "bench-events")]
  fn marks_acknowledge_session_writes() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("trace.jsonl");
    let trace = Arc::new(crate::trace::Log::open(&path)?);
    let (mut bridge, wakes) = launch(Vec::new(), None, None)?;
    wait(&bridge, &wakes, |frame| frame.complete)?;
    let (sender, receiver) = mpsc::channel();
    let ticket = bridge.send(Input::Mark {
      name: "phase".into(),
      trace: Arc::clone(&trace),
      ack: Some(sender),
    });
    assert_eq!(receiver.recv_timeout(Duration::from_secs(10))?, Ok(()));
    let frame = wait(&bridge, &wakes, |frame| frame.metrics.ticket >= ticket)?;
    assert!(!frame.metrics.busy);
    bridge.close()?;
    drop(trace);
    let rows = std::fs::read_to_string(path)?;
    let marks: Vec<serde_json::Value> = rows
      .lines()
      .map(serde_json::from_str)
      .collect::<std::result::Result<_, _>>()?;
    assert!(
      marks.iter().any(|row| row["kind"] == "mark" && row["name"] == "phase")
    );
    Ok(())
  }

  #[test]
  fn unchanged_motion_skips_frames() -> Result<()> {
    let mut screen = sized();
    let (_root, _, mut session) = opened(&["body"], screen)?;
    let mut cache = Cache::new(BUDGET);
    frame(&mut session, &mut cache, screen)?;
    let mut chores = Chores::new(Instant::now());
    let pending = AtomicBool::new(false);
    let drawn = screen;
    let moved = |x| mouse(MouseEventKind::Moved, (x, 300), drawn);
    let mut inputs = vec![(moved(500), true)];
    inputs.extend((501..521).map(|x| (moved(x), false)));
    inputs.push((Input::Leave, true));
    inputs.push((Input::Leave, false));
    inputs.push((moved(500), true));
    for (input, changed) in inputs {
      assert_eq!(
        drain(
          &mut session,
          std::iter::once(input),
          &mut chores,
          &mut screen,
          &pending,
        ),
        Some(changed)
      );
    }
    Ok(())
  }

  #[test]
  fn home_motion_skips_frames() {
    use vpdf_model::input::MouseButton;
    let mut session = Session::new(None);
    let mut screen = sized();
    ensure_home(&mut session, screen);
    for kind in [MouseEventKind::Moved, MouseEventKind::Drag(MouseButton::Left)]
    {
      assert!(!apply(
        &mut session,
        mouse(kind, (50, 100), screen),
        &mut screen
      ));
    }
  }

  #[test]
  fn unrelated_wakes_keep_animation_deadline() -> Result<()> {
    let (_root, _, mut session) = opened(&["first", "second"], sized())?;
    let start = Instant::now();
    session.app().unwrap().reader.animate_to(100.0, start);
    let mut due = start + ANIMATION;
    let mut chores = Chores::new(start);
    let mut screen = sized();
    let pending = AtomicBool::new(true);
    for step in 1..8 {
      assert_eq!(
        drain(
          &mut session,
          std::iter::once(Input::Wake),
          &mut chores,
          &mut screen,
          &pending
        ),
        Some(false)
      );
      assert!(!pending.load(Ordering::Acquire));
      assert!(!animate(&mut session, &mut due, start + ANIMATION * step / 8));
      assert_eq!(session.app().unwrap().reader.scroll, 0.0);
      assert_eq!(due, start + ANIMATION);
    }
    assert!(animate(&mut session, &mut due, start + ANIMATION));
    let scroll = session.app().unwrap().reader.scroll;
    assert!(scroll > 0.0);
    assert!(!animate(&mut session, &mut due, start + ANIMATION));
    assert_eq!(session.app().unwrap().reader.scroll, scroll);
    session.unsaved = false;
    assert_eq!(
      wait_time(&mut session, &chores, due, start + ANIMATION),
      ANIMATION
    );
    for now in [due, due + ANIMATION] {
      assert_eq!(wait_time(&mut session, &chores, due, now), Duration::ZERO);
    }
    let deadline = due;
    assert!(animate(
      &mut session,
      &mut due,
      deadline + Duration::from_millis(1)
    ));
    assert_eq!(due, deadline + ANIMATION);
    session.app().unwrap().reader.cancel_scroll();
    assert!(!animate(&mut session, &mut due, start + ANIMATION));
    assert_eq!(
      wait_time(&mut session, &chores, due, start),
      Duration::from_secs(2)
    );
    Ok(())
  }

  fn key(code: KeyCode) -> Input {
    Input::Key(KeyEvent::new(code, KeyModifiers::NONE))
  }

  fn assert_sessions_match(left: &mut Session, right: &mut Session) {
    assert_eq!(left.active, right.active);
    assert_eq!(left.tab().focus, right.tab().focus);
    assert_eq!(left.tab().panes.len(), right.tab().panes.len());
    assert_eq!(left.pointer.holding(), right.pointer.holding());
    assert_eq!(left.history.commands(), right.history.commands());
    for (left, right) in left.tab().panes.iter().zip(&right.tab().panes) {
      assert_eq!(
        left.document().unwrap().reader.cursor,
        right.document().unwrap().reader.cursor
      );
      assert_eq!(
        left.document().unwrap().reader.scroll,
        right.document().unwrap().reader.scroll
      );
      assert_eq!(
        left.document().unwrap().reader.mode(),
        right.document().unwrap().reader.mode()
      );
      assert_eq!(
        left.document().unwrap().reader.selection,
        right.document().unwrap().reader.selection
      );
      assert_eq!(
        left.document().unwrap().layout.zoom(),
        right.document().unwrap().layout.zoom()
      );
      assert_eq!(
        left.document().unwrap().error,
        right.document().unwrap().error
      );
      assert_eq!(
        left
          .document()
          .unwrap()
          .overlays
          .prompt()
          .map(vpdf_ui::prompt::Prompt::text),
        right
          .document()
          .unwrap()
          .overlays
          .prompt()
          .map(vpdf_ui::prompt::Prompt::text)
      );
      assert!(
        left.document().unwrap().request.is_none()
          && right.document().unwrap().request.is_none()
      );
    }
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn normalized_adapter_inputs_match() -> Result<()> {
    use super::super::{Act, run};
    use vpdf_model::input::{Button, Mouse, MouseKind, Spot};
    use vpdf_model::text::PageId;
    let path = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
    let mut screen = sized();
    let mut terminal = Session::new(None);
    let mut native = native();
    for session in [&mut terminal, &mut native] {
      session.open(&path, true, screen)?;
    }
    let key_pair = |code, modifiers| {
      let key = KeyEvent::new(code, modifiers);
      (Act::Key(key), Input::Key(key))
    };
    let mut inputs = Vec::new();
    let mut pages = Vec::new();
    for (command, page) in [(":vsplit", 0), (":2", 1), (":1", 0)] {
      inputs.extend(
        command
          .chars()
          .map(|ch| key_pair(KeyCode::Char(ch), KeyModifiers::NONE)),
      );
      inputs.push(key_pair(KeyCode::Enter, KeyModifiers::NONE));
      pages.push((inputs.len() - 1, PageId(page)));
    }
    for forward in [false, true] {
      let button = if forward { Button::FORWARD } else { Button::BACK };
      inputs.push((
        Act::Mouse(Mouse {
          kind: MouseKind::Press(button),
          at: Spot::Pixel { x: 100, y: 160 },
          modifiers: KeyModifiers::NONE,
        }),
        Input::History { forward, pixels: (100, 160), screen },
      ));
      pages.push((inputs.len() - 1, PageId(usize::from(!forward))));
    }
    for kind in [
      MouseEventKind::Moved,
      MouseEventKind::Down(vpdf_model::input::MouseButton::Left),
      MouseEventKind::Drag(vpdf_model::input::MouseButton::Left),
      MouseEventKind::Up(vpdf_model::input::MouseButton::Left),
      MouseEventKind::ScrollDown,
      MouseEventKind::ScrollUp,
    ] {
      let event =
        MouseEvent { kind, column: 60, row: 8, modifiers: KeyModifiers::SHIFT };
      let mut report = Mouse::from(event);
      report.at = Spot::Pixel { x: 600, y: 160 };
      inputs.push((
        Act::Mouse(report),
        Input::Mouse { event, pixels: (600, 160), screen },
      ));
    }
    inputs.push((Act::Leave, Input::Leave));
    inputs.push(key_pair(KeyCode::Char('/'), KeyModifiers::NONE));
    inputs.push((
      Act::Paste("章a\u{301}".into()),
      Input::Paste("章a\u{301}".into()),
    ));
    let pasted = inputs.len() - 1;
    inputs.push(key_pair(KeyCode::Char('_'), KeyModifiers::CONTROL));
    let undone = inputs.len() - 1;
    inputs.push(key_pair(KeyCode::Esc, KeyModifiers::NONE));
    for (step, (term_input, native_input)) in inputs.into_iter().enumerate() {
      run::apply(&mut terminal, term_input, &mut Vec::new());
      apply(&mut native, native_input, &mut screen);
      assert!(terminal.app().unwrap().error.is_none(), "terminal step {step}");
      assert!(native.app().unwrap().error.is_none(), "native step {step}");
      if let Some((_, page)) = pages.iter().find(|(at, _)| *at == step) {
        assert_eq!(terminal.tab().panes.len(), 2, "step {step}");
        assert_eq!(
          terminal.app().unwrap().reader.cursor.page,
          *page,
          "step {step}"
        );
      }
      if step == pasted || step == undone {
        let expected = if step == pasted { "章a\u{301}" } else { "" };
        assert_eq!(
          terminal.app().unwrap().overlays.prompt().unwrap().text(),
          expected
        );
      }
      assert_sessions_match(&mut terminal, &mut native);
    }
    assert_eq!(terminal.tab().focus, 0);
    assert!(terminal.app().unwrap().overlays.prompt().is_none());
    Ok(())
  }

  #[test]
  fn guarded_inputs_keep_requests() -> Result<()> {
    let mut screen = sized();
    let (_root, path, mut session) = opened(&["body"], screen)?;
    let stale = Screen { columns: screen.columns - 1, ..screen };
    session.app().unwrap().request =
      Some(super::super::Request::Perf(Some(true)));
    for input in [
      Input::Leave,
      Input::Focus(false),
      Input::Resize(screen),
      Input::History { forward: false, pixels: (20, 20), screen: stale },
      mouse(MouseEventKind::Moved, (20, 20), stale),
    ] {
      apply(&mut session, input, &mut screen);
      assert!(!session.state.perf);
      assert!(session.app().unwrap().request.is_some());
    }
    session.unlock_result(
      Err(super::super::PasswordRequired.into()),
      Some(Operation::Open(path)),
    )?;
    session
      .pending_keys
      .push_back(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE));
    for input in [
      Input::Paste("secret".into()),
      key(KeyCode::Char('x')),
      Input::History { forward: true, pixels: (20, 20), screen },
    ] {
      apply(&mut session, input, &mut screen);
      assert!(!session.state.perf);
      assert!(session.app().unwrap().request.is_some());
      assert_eq!(session.pending_keys.len(), 1);
      assert!(!resume_keys(&mut session));
    }
    assert_eq!(session.unlock.as_ref().unwrap().prompt.text(), "secretx");
    Ok(())
  }

  pub(super) fn protected(
    path: &std::path::Path,
  ) -> Result<tempfile::NamedTempFile> {
    let mut source = vpdf_pdf::document::Document::open(path, "")?;
    crate::encryption::encrypt(&mut source.native)
  }

  #[test]
  fn mouse_settles_once() -> Result<()> {
    use super::super::{Act, Request, run};
    use vpdf_model::input::{Mouse, MouseKind, Spot};
    let root = tempfile::tempdir()?;
    for native in [false, true] {
      let mut session =
        if native { self::native() } else { Session::new(None) };
      let mut screen = sized();
      for name in ["first.pdf", "second.pdf"] {
        let path = root.path().join(name);
        crate::fixtures::pdf(&path, &["body"], "")?;
        session.open(&path, true, screen)?;
      }
      session.tabs[0].panes[0].document_mut().unwrap().quit = true;
      session.app().unwrap().request = Some(Request::TabClose(None));
      if native {
        apply(
          &mut session,
          mouse(MouseEventKind::Moved, (0, 0), screen),
          &mut screen,
        );
      } else {
        run::apply(
          &mut session,
          Act::Mouse(Mouse {
            kind: MouseKind::Moved,
            at: Spot::Pixel { x: 0, y: 0 },
            modifiers: KeyModifiers::NONE,
          }),
          &mut Vec::new(),
        );
      }
      assert_eq!(session.tabs.len(), 1);
      assert!(session.app().unwrap().quit);
      assert!(session.app().unwrap().error.is_none());
    }
    Ok(())
  }

  #[test]
  fn leave_preserves_frontend_policy() -> Result<()> {
    use super::super::{Act, Request, run};
    use vpdf_model::input::MouseButton;
    let (_root, path) = temp_pdf(&["body"])?;
    for native in [false, true] {
      let mut session =
        if native { self::native() } else { Session::new(None) };
      let mut screen = sized();
      session.open(&path, true, screen)?;
      for kind in
        [MouseEventKind::Moved, MouseEventKind::Down(MouseButton::Left)]
      {
        session.mouse(MouseEvent::new(kind, 10, 8))?;
      }
      assert!(session.pointer.holding());
      assert!(session.pointer.target().is_some());
      session.app().unwrap().request = Some(Request::SetMark('a'));
      if native {
        apply(&mut session, Input::Leave, &mut screen);
      } else {
        assert!(run::apply(&mut session, Act::Leave, &mut Vec::new()));
      }
      assert!(session.pointer.target().is_none());
      assert_eq!(session.pointer.holding(), !native);
      assert_eq!(session.app().unwrap().request.is_some(), native);
      assert_eq!(session.state.marks.contains_key(&'a'), !native);
      assert!(session.app().unwrap().error.is_none());
    }
    Ok(())
  }

  /// A frame the window never took hands its one-shot fields on.
  #[test]
  fn publish_merges_untaken_frames() {
    let frame = |owner: u32, clipboard: Option<&str>| Frame {
      ui: Arc::default(),
      effects: Vec::new(),
      metrics: Metrics::default(),
      ui_font_size: 16.0,
      screen: sized(),
      scene: Scene::new(),
      images: Vec::new(),
      owners: Vec::new(),
      reloaded: vec![TileOwner::new(owner)],
      title: String::new(),
      cursor: vpdf_model::style::Shape::Default,
      complete: true,
      content: false,
      quit: false,
      clipboard: clipboard.map(str::to_owned),
    };
    let latest = Mutex::new(None);
    publish(&latest, frame(1, Some("copied")));
    publish(&latest, frame(2, None));
    let shown = latest.lock().unwrap().take().unwrap();
    assert_eq!(shown.reloaded, [TileOwner::new(2), TileOwner::new(1)]);
    assert_eq!(shown.clipboard.as_deref(), Some("copied"));
    publish(&latest, frame(3, None));
    let next = latest.lock().unwrap().take().unwrap();
    assert_eq!(next.reloaded, [TileOwner::new(3)]);
    assert_eq!(next.clipboard, None);
  }

  #[test]
  fn passwords_resume_open_operations() -> Result<()> {
    use super::super::{Axis, Mark, Position, Request, Window};
    let (_root, source) = temp_pdf(&["first", "second"])?;
    let encrypted = protected(&source)?;
    let path = std::fs::canonicalize(encrypted.path())?;
    for request in [
      Request::Open { path: path.clone(), tab: false },
      Request::Open { path: path.clone(), tab: true },
      Request::Window {
        command: Window::Split(Axis::Columns),
        count: 1,
        explicit: false,
        file: Some(path.clone().into()),
      },
      Request::GoMark('A', true),
      Request::PanelMark { name: 'A', focus: false },
    ] {
      let mut screen = sized();
      let mut session = native();
      session.open(&source, true, screen)?;
      let original = session.app().unwrap().id;
      let position =
        Position { page: vpdf_model::text::PageId(1), ..Position::default() };
      session.state.marks.insert('A', Mark::new(path.clone(), position));
      session.app().unwrap().request = Some(request.clone());
      session.settle()?;
      assert!(session.unlock.is_some(), "{request:?}");
      assert_eq!(session.app().unwrap().id, original);
      apply(&mut session, Input::Paste("wrong".into()), &mut screen);
      apply(&mut session, key(KeyCode::Enter), &mut screen);
      assert!(session.unlock.as_ref().unwrap().retry);
      assert_eq!(session.app().unwrap().id, original);
      let resized = Screen { columns: 120, rows: 45, ..screen };
      apply(&mut session, Input::Resize(resized), &mut screen);
      apply(&mut session, Input::Paste("test-reader".into()), &mut screen);
      apply(&mut session, key(KeyCode::Enter), &mut screen);
      assert!(session.unlock.is_none(), "{request:?}");
      assert!(session.secret.is_none());
      resumed(&mut session, &request, original, &path, position);
      assert_eq!(session.history.commands(), &[] as &[String]);
      assert_eq!(session.history.searches(), &[] as &[String]);
    }
    Ok(())
  }

  fn resumed(
    session: &mut Session,
    request: &super::super::Request,
    original: u32,
    path: &std::path::Path,
    position: super::super::Position,
  ) {
    use super::super::Request;
    let target = session.existing(path).unwrap();
    match request {
      Request::Open { tab: false, .. } => {
        assert_eq!(session.tabs.len(), 1);
        assert_ne!(session.app().unwrap().id, original);
      }
      Request::Window { .. } => {
        assert_eq!(session.tabs.len(), 1);
        assert_eq!(session.tab().panes.len(), 2);
        assert_eq!(session.app().unwrap().path, path);
      }
      Request::PanelMark { .. } => {
        assert_eq!(session.app().unwrap().id, original);
        assert_eq!(session.tab().panes.len(), 2);
      }
      _ => assert_eq!(session.tabs.len(), 2),
    }
    if matches!(request, Request::GoMark(..) | Request::PanelMark { .. }) {
      assert_eq!(
        session.tabs[target.0].panes[target.1]
          .document()
          .unwrap()
          .reader
          .cursor
          .page,
        position.page
      );
    }
  }

  #[test]
  fn cancelled_open_keeps_forgotten_history() -> Result<()> {
    let (_root, source) = temp_pdf(&["first"])?;
    let encrypted = protected(&source)?;
    let path = std::fs::canonicalize(encrypted.path())?;
    let store = source.with_extension("json");
    let mut screen = sized();
    let mut session = native();
    session.store = Some(store.clone());
    session.state.opened(crate::state::tests::entry(
      path.to_str().unwrap(),
      0,
      1,
    ));
    session.forget_recent(std::slice::from_ref(&path))?;
    apply(&mut session, Input::Open(path), &mut screen);
    assert!(session.unlock.is_some());
    apply(&mut session, Input::Paste("wrong".into()), &mut screen);
    apply(&mut session, key(KeyCode::Enter), &mut screen);
    assert!(session.unlock.as_ref().unwrap().retry);
    apply(&mut session, key(KeyCode::Esc), &mut screen);
    assert!(session.unlock.is_none());
    session.finish()?;
    assert_eq!(crate::state::State::load(&store).recent, []);
    Ok(())
  }

  #[test]
  fn password_prompt_lifecycle() -> Result<()> {
    let (_root, path) = temp_pdf(&["first"])?;
    let encrypted = protected(&path)?;
    let mut screen = sized();
    let mut session = native();
    ensure_home(&mut session, screen);
    apply(&mut session, Input::Open(encrypted.path().into()), &mut screen);
    assert!(session.recent().is_some());
    apply(&mut session, Input::Paste("secret".into()), &mut screen);
    let mut cache = Cache::new(BUDGET);
    let drawn = frame(&mut session, &mut cache, screen)?;
    assert!(masked(&drawn));
    assert!(!shows(&drawn.scene, "secret"));
    apply(&mut session, Input::Focus(false), &mut screen);
    assert_eq!(session.unlock.as_ref().unwrap().prompt.text(), "secret");
    apply(&mut session, Input::Open(path.clone()), &mut screen);
    assert_eq!(session.native_opens.front(), Some(&path.into()));
    assert!(session.focused().is_recent());
    apply(&mut session, key(KeyCode::Esc), &mut screen);
    assert!(session.unlock.is_none());
    assert!(session.focused().is_recent());
    assert!(session.recent().is_some() && !session.quit);
    let (mut bridge, wakes) =
      launch(vec![encrypted.path().into()], None, None)?;
    wait(&bridge, &wakes, masked)?;
    bridge.close()
  }

  #[test]
  fn protected_keys_resume() -> Result<()> {
    use super::super::Request;
    let (_root, source) = temp_pdf(&["first", "second"])?;
    let encrypted = protected(&source)?;
    for cancel in [false, true] {
      let mut screen = sized();
      let mut session = native();
      session.open(&source, true, screen)?;
      let mut keys: Vec<_> = format!(":e {}", encrypted.path().display())
        .chars()
        .map(|ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
        .collect();
      keys.extend(
        [
          KeyCode::Enter,
          KeyCode::Char('g'),
          KeyCode::Char('g'),
          KeyCode::Char('G'),
        ]
        .map(|code| KeyEvent::new(code, KeyModifiers::NONE)),
      );
      session.app().unwrap().request = Some(Request::Keys(keys));
      session.settle()?;
      assert!(session.unlock.is_some());
      assert_eq!(session.pending_keys.len(), 3);
      if cancel {
        apply(&mut session, key(KeyCode::Esc), &mut screen);
        assert!(!resume_keys(&mut session));
        assert_eq!(
          session.app().unwrap().path,
          std::fs::canonicalize(&source)?
        );
      } else {
        apply(&mut session, Input::Paste("test-reader".into()), &mut screen);
        apply(&mut session, key(KeyCode::Enter), &mut screen);
        assert!(resume_keys(&mut session));
        assert_eq!(
          session.app().unwrap().reader.cursor.page,
          vpdf_model::text::PageId(1)
        );
      }
      assert_eq!(session.pending_keys.len(), 0);
    }
    Ok(())
  }

  #[test]
  fn close_keys_stop() -> Result<()> {
    let (_root, _, mut session) = opened(&["first"], sized())?;
    session.app().unwrap().request = Some(super::super::Request::Keys(
      [
        KeyCode::Char(':'),
        KeyCode::Char('q'),
        KeyCode::Enter,
        KeyCode::Char('j'),
      ]
      .map(|code| KeyEvent::new(code, KeyModifiers::NONE))
      .into(),
    ));
    session.settle()?;
    assert!(session.tabs.is_empty());
    Ok(())
  }

  #[test]
  fn close_interrupts_a_running_search() -> Result<()> {
    let pages = vec!["text"; 4000];
    let (directory, path) = temp_pdf(&pages)?;
    let (mut bridge, wakes) =
      launch(vec![path], Some(directory.path().join("state.json")), None)?;
    bridge.send(Input::Resize(sized()));
    for input in [
      key(KeyCode::Char('/')),
      Input::Paste("absent".into()),
      key(KeyCode::Enter),
    ] {
      bridge.send(input);
    }
    wait(&bridge, &wakes, |frame| frame.metrics.searching)?;
    let started = Instant::now();
    bridge.close()?;
    assert!(
      started.elapsed() < Duration::from_secs(1),
      "{:?}",
      started.elapsed()
    );
    Ok(())
  }

  #[test]
  fn home_reserves_perf_and_last_close_quits() -> Result<()> {
    let (_root, path) = temp_pdf(&["first"])?;
    let mut screen = sized();
    let mut session = native();
    session.state.set_perf(true);
    ensure_home(&mut session, screen);
    apply(&mut session, Input::Focus(true), &mut screen);
    assert_eq!(
      session.recent().unwrap().view.screen(),
      session.content(screen)
    );
    assert!(session.recent().unwrap().view.screen().origin.1 > 0);
    apply(
      &mut session,
      Input::Open(PathBuf::from("/missing.pdf")),
      &mut screen,
    );
    assert!(session.recent().is_some() && !session.quit);
    apply(&mut session, Input::Open(path), &mut screen);
    assert!(session.recent().is_none());
    apply(&mut session, key(KeyCode::Char('q')), &mut screen);
    assert!(session.tabs.is_empty() && session.quit);
    Ok(())
  }

  #[test]
  fn help_theme_preserves_native_tiles() -> Result<()> {
    let mut screen = sized();
    let mut session = native();
    session.request_help(
      "config",
      vpdf_ui::topics::Invocation::Home,
      screen,
    )?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while !session.panes().any(|app| app.manual.is_some()) {
      session.help_updates(&mut std::io::sink());
      assert!(Instant::now() < deadline, "Help generation timed out");
      thread::sleep(Duration::from_millis(2));
    }
    session.split(
      super::super::Axis::Columns,
      None,
      super::super::Place::Before,
    )?;
    let owners: Vec<_> =
      session.tab().panes.iter().map(|app| TileOwner::new(app.id)).collect();
    let mut cache = Cache::new(BUDGET);
    completions(&mut session, &mut cache, &mut Vec::new());
    for &owner in &owners {
      cache.insert(
        TileId { owner, key: vpdf_model::tile::TileKey::default() },
        Arc::new(vec![255, 0, 0]),
        3,
      );
    }
    let original = session.app().unwrap().manual.clone().unwrap();
    assert!(apply(
      &mut session,
      Input::SystemPalette(vpdf_model::theme::Palette::light()),
      &mut screen
    ));
    assert_eq!(session.system_palette, vpdf_model::theme::Palette::light());
    while !session.help_updates(&mut std::io::sink()) {
      assert!(Instant::now() < deadline, "Help refresh timed out");
      thread::sleep(Duration::from_millis(2));
    }
    assert!(Arc::ptr_eq(
      session.app().unwrap().manual.as_ref().unwrap(),
      &original
    ));
    assert_eq!(session.app().unwrap().document_generation, 0);
    let mut reloaded = Vec::new();
    completions(&mut session, &mut cache, &mut reloaded);
    assert_eq!(reloaded, [] as [TileOwner; 0]);
    for owner in owners {
      assert!(
        cache
          .get(&TileId { owner, key: vpdf_model::tile::TileKey::default() })
          .is_some()
      );
    }
    Ok(())
  }

  #[test]
  fn picker_open_destinations() -> Result<()> {
    let root = tempfile::tempdir()?;
    let first = root.path().join("first.pdf");
    let second = root.path().join("second.pdf");
    crate::fixtures::pdf(&first, &["first"], "")?;
    crate::fixtures::pdf(&second, &["second"], "")?;
    for tab in [false, true] {
      let screen = sized();
      let mut session = native();
      picked_path(&mut session, first.clone(), tab, screen)?;
      assert_eq!(session.tabs.len(), 1);
      let original = session.app().unwrap().id;
      session.picker = Some(crate::programs::Picker::exited(
        tab,
        Some(second.to_str().unwrap()),
      )?);
      assert!(picker_update(&mut session, screen));
      assert!(session.picker.is_none());
      assert_eq!(session.tabs.len(), if tab { 2 } else { 1 });
      assert_eq!(session.app().unwrap().path, std::fs::canonicalize(&second)?);
      if tab {
        assert_eq!(session.tabs[0].panes[0].id, original);
      }
      picked_path(&mut session, second.clone(), tab, screen)?;
      assert_eq!(session.tabs.len(), if tab { 2 } else { 1 });
    }
    Ok(())
  }

  #[test]
  fn picker_cancel_and_failure() -> Result<()> {
    let (_root, source) = temp_pdf(&["first"])?;
    for (tab, empty) in
      [(false, false), (false, true), (true, false), (true, true)]
    {
      for result in [Some(""), None] {
        let screen = sized();
        let mut session = native();
        if empty {
          session.recent_tab(screen).unwrap();
        } else {
          session.open(&source, true, screen)?;
          session.show_recent()?;
        }
        let original = session.tabs.first().map(|t| t.panes[0].id);
        session.picker = Some(crate::programs::Picker::exited(tab, result)?);
        assert!(picker_update(&mut session, screen));
        assert!(session.picker.is_none());
        assert!(session.unlock.is_none());
        assert_eq!(session.tabs.first().map(|t| t.panes[0].id), original);
        if empty {
          assert_eq!(
            session.recent().unwrap().view.error().is_some(),
            result.is_none()
          );
        } else {
          assert!(session.focused().is_recent());
        }
      }
    }
    Ok(())
  }

  #[test]
  fn picker_password_retries() -> Result<()> {
    let (_root, source) = temp_pdf(&["first"])?;
    let encrypted = protected(&source)?;
    let path = std::fs::canonicalize(encrypted.path())?;
    for (tab, empty) in
      [(false, false), (false, true), (true, false), (true, true)]
    {
      for cancel in [false, true] {
        let mut screen = sized();
        let mut session = native();
        if !empty {
          session.open(&source, true, screen)?;
        }
        let original = session.tabs.first().map(|t| t.panes[0].id);
        session.picker = Some(crate::programs::Picker::exited(
          tab,
          Some(path.to_str().unwrap()),
        )?);
        assert!(picker_update(&mut session, screen));
        assert!(session.unlock.is_some());
        apply(&mut session, Input::Paste("wrong".into()), &mut screen);
        apply(&mut session, key(KeyCode::Enter), &mut screen);
        assert!(session.unlock.as_ref().unwrap().retry);
        assert_eq!(session.tabs.first().map(|t| t.panes[0].id), original);
        if cancel {
          apply(&mut session, key(KeyCode::Esc), &mut screen);
          assert_eq!(session.tabs.first().map(|t| t.panes[0].id), original);
        } else {
          apply(&mut session, Input::Paste("test-reader".into()), &mut screen);
          apply(&mut session, key(KeyCode::Enter), &mut screen);
          assert_eq!(session.tabs.len(), if !empty && tab { 2 } else { 1 });
          assert_eq!(session.app().unwrap().path, path);
        }
        assert!(session.unlock.is_none());
      }
    }
    Ok(())
  }

  #[test]
  fn native_perf_accounting() -> Result<()> {
    let root = tempfile::tempdir()?;
    let store = root.path().join("state.json");
    let mut state = crate::state::State::default();
    state.set_perf(true);
    state.save(&store, &[], &[])?;
    let (mut bridge, wakes) = launch(Vec::new(), Some(store), None)?;
    wait(&bridge, &wakes, |frame| frame.content)?;
    let sampled = wait(&bridge, &wakes, |frame| {
      !frame.content
        && frame.scene.iter().any(|op| {
          matches!(op,
        Op::Text(text) if text.as_str().contains("RSS ")
          && !text.as_str().contains("RSS —"))
        })
    })?;
    assert!(shows(&sampled.scene, "FPS 0"));
    bridge.send(Input::Presented { content: true, bytes: 2048 });
    wait(&bridge, &wakes, |frame| shows(&frame.scene, "Upload 2.0K/s"))?;
    bridge.send(key(KeyCode::Char(':')));
    bridge.send(Input::Paste("perf off".into()));
    bridge.send(key(KeyCode::Enter));
    wait(&bridge, &wakes, |frame| !shows(&frame.scene, "Upload "))?;
    bridge.close()
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn encrypted_corpus_renders() -> Result<()> {
    let path = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
    let encrypted = protected(&path)?;
    let (mut bridge, wakes) =
      launch(vec![encrypted.path().into()], None, None)?;
    wait(&bridge, &wakes, masked)?;
    bridge.send(Input::Paste("test-reader".into()));
    bridge.send(key(KeyCode::Enter));
    wait(&bridge, &wakes, |frame| frame.complete && !frame.images.is_empty())?;
    bridge.close()
  }

  #[test]
  fn launch_page_overrides_state() -> Result<()> {
    let root = tempfile::tempdir()?;
    let first = root.path().join("first.pdf");
    let second = root.path().join("second.pdf");
    crate::fixtures::pdf(&first, &["one", "two", "three"], "/Rotate 90")?;
    crate::fixtures::pdf(&second, &["other"], "")?;
    let store = root.path().join("state.json");
    let canonical = std::fs::canonicalize(&first)?;
    for (page, expected) in
      [(None, 2), (Some(1), 0), (Some(2), 1), (Some(3), 2)]
    {
      let mut saved = Session::new(Some(store.clone()));
      saved.open(&first, true, sized())?;
      saved.start_at(3)?;
      saved.finish()?;
      drop(saved);
      let (mut bridge, wakes) =
        launch(vec![first.clone(), second.clone()], Some(store.clone()), page)?;
      let frame = wait(&bridge, &wakes, |frame| {
        frame.complete && !frame.images.is_empty()
      })?;
      assert_eq!(frame.title, "first.pdf");
      bridge.close()?;
      let state = crate::state::State::load(&store);
      assert_eq!(
        state.entry(&canonical).expect("saved document").cursor.page.0,
        expected,
        "{page:?}"
      );
    }
    Ok(())
  }

  #[test]
  fn launch_page_waits_for_password() -> Result<()> {
    let root = tempfile::tempdir()?;
    let first = root.path().join("first.pdf");
    let second = root.path().join("second.pdf");
    crate::fixtures::pdf(&first, &["one", "two", "three"], "/Rotate 90")?;
    crate::fixtures::pdf(&second, &["other"], "")?;
    let encrypted = protected(&first)?;
    let store = root.path().join("state.json");
    let (mut bridge, wakes) = launch(
      vec![encrypted.path().into(), second],
      Some(store.clone()),
      Some(3),
    )?;
    wait(&bridge, &wakes, masked)?;
    bridge.send(Input::Paste("test-reader".into()));
    bridge.send(key(KeyCode::Enter));
    wait(&bridge, &wakes, |frame| {
      frame.complete
        && frame.images.iter().any(|image| image.id.key.page.0 == 2)
    })?;
    bridge.close()?;
    let state = crate::state::State::load(&store);
    let path = std::fs::canonicalize(encrypted.path())?;
    assert_eq!(state.entry(&path).expect("saved document").cursor.page.0, 2);
    Ok(())
  }

  #[test]
  fn opens_renders_navigates_and_saves() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("native.pdf");
    crate::fixtures::pdf(&path, &["first", "second"], "")?;
    let store = root.path().join("state.json");
    let (mut bridge, wakes) =
      launch(vec![path.clone()], Some(store.clone()), None)?;
    let first = wait(&bridge, &wakes, |frame| {
      frame.complete && !frame.images.is_empty()
    })?;
    assert!(first.images.iter().all(|image| image.pixels.len()
      == image.id.key.width as usize * image.id.key.height as usize * 3));
    let screen = crate::screen::pixels(60, 20, 12, 24);
    bridge.send(Input::Resize(screen));
    bridge.send(key(KeyCode::Char('G')));
    let last = wait(&bridge, &wakes, |frame| {
      frame.screen == screen
        && frame.complete
        && frame.images.iter().any(|image| image.id.key.page.0 == 1)
    })?;
    assert_eq!(first.owners, last.owners);
    bridge.close()?;
    let state = crate::state::State::load(&store);
    assert!(state.entry(&std::fs::canonicalize(&path)?).is_some());
    bridge.close()?;
    let (mut home, wakes) = launch(Vec::new(), Some(store), None)?;
    let card = wait(&home, &wakes, |frame| {
      frame.scene.iter().any(|op| matches!(op, Op::Picture(_)))
    })?;
    assert_eq!(card.owners, [] as [TileOwner; 0]);
    home.close()?;
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn retained_panes_match_terminal_views() -> Result<()> {
    use crate::app::App;
    use vpdf_model::document::Target;
    use vpdf_model::text::{PageId, Position};
    use vpdf_terminal::graphics::Graphics;
    use vpdf_ui::keys::{Action, Input, PanelKind};
    let path = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
    let screen = cells(120, 40);
    for view in ["page", "text", "panel", "overview", "preview", "clues"] {
      let setup = || -> Result<App> {
        let mut app = App::with_screen(&path, "", view == "text", screen)?;
        match view {
          "panel" => {
            app.action(Input::plain(Action::Panel(PanelKind::Version)))?;
            assert!(app.panel.is_some());
          }
          "overview" => app.open_overview()?,
          "preview" => {
            app.show_preview(Target::Page(PageId(1)), Position::default())?;
          }
          "clues" => {
            app.overlays.clues_mut().show(vec![("gg".into(), "top".into())]);
          }
          _ => {}
        }
        Ok(app)
      };
      let mut streamed = setup()?;
      let mut retained_app = setup()?;
      let mut streaming = Graphics::default();
      let mut retained = Graphics::default();
      for _ in 0..2 {
        let mut expected = Vec::new();
        streaming.begin(&mut expected)?;
        streamed.draw_pane(&mut streaming, &mut expected)?;
        streaming.end(&mut expected)?;
        let mut recorder = Recorder::new(|_| false);
        retained_app.scene_pane(&mut recorder, streaming.crop_quirk())?;
        let mut actual = Vec::new();
        retained.begin(&mut actual)?;
        let mut painter = vpdf_terminal::painter::Painter::new(&mut actual)
          .with_graphics(&mut retained);
        painter.paint(&recorder.into_scene());
        painter.finish()?;
        retained.end(&mut actual)?;
        assert_eq!(actual, expected, "{view}");
      }
    }
    Ok(())
  }

  fn composition_case(
    path: &std::path::Path,
    view: &str,
    focus: usize,
  ) -> Result<Session> {
    use vpdf_ui::keys::{Action, Input};
    let mut session = Session::new(None);
    session.open(path, true, sized())?;
    let copied = session.copy_pane(sized())?;
    session.insert_tab(copied)?;
    for command in ["vsplit", "split"] {
      session.app().unwrap().command(command)?;
      session.settle()?;
    }
    session.focus(session.active, focus)?;
    for app in session.tabs[session.active]
      .panes
      .iter_mut()
      .filter_map(Pane::document_mut)
    {
      app.close_panel()?;
      match view {
        "page" | "covered" => {}
        "overview" => app.open_overview()?,
        _ => app.text_mode = true,
      }
    }
    if view == "covered" {
      let count = session.tab().panes.len();
      for index in 0..count {
        session.focus(session.active, index)?;
        session.show_recent()?;
      }
      session.focus(session.active, focus)?;
    }
    if view == "prompt" {
      session.app().unwrap().action(Input::plain(Action::Command))?;
      session.app().unwrap().overlays.prompt_mut().unwrap().paste("set ");
    }
    let screen = match view {
      "tiny" => Screen { columns: 3, rows: 2, ..sized() },
      "empty" => Screen { columns: 0, rows: 0, ..sized() },
      _ => sized(),
    };
    session.resize(screen)?;
    session.unmeasured = Some(super::super::Notice::new(
      screen.rows,
      screen.columns,
      "Resize pending".into(),
    ));
    Ok(session)
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn retained_sessions_match_terminal_frames() -> Result<()> {
    use crate::trace::Drawable;
    use std::io::Write;
    use vpdf_model::style::Role;
    use vpdf_terminal::{
      graphics::Graphics, painter::Painter, style::sgr, terminal,
    };
    let path = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
    let cases =
      ["page", "text", "covered", "overview", "prompt", "tiny", "empty"];
    for (view, focus) in
      cases.into_iter().flat_map(|view| (0..3).map(move |focus| (view, focus)))
    {
      let mut streamed = composition_case(&path, view, focus)?;
      let mut recorded = composition_case(&path, view, focus)?;
      let mut legacy = composition_case(&path, view, focus)?;
      let mut graphics = Graphics::default();
      let mut cache = Cache::new(BUDGET);
      for _ in 0..2 {
        let mut expected = Vec::new();
        streamed.draw_frame(&mut expected)?;
        let screen = recorded.screen.unwrap();
        let mut retained = frame(&mut recorded, &mut cache, screen)?;
        assert_eq!(
          retained.scene.remove(0),
          Op::theme(
            vpdf_model::theme::Colors::Palette(recorded.system_palette),
            vpdf_model::theme::Opacity::default()
          )
        );
        let mut actual = Vec::new();
        write!(actual, "{}{}", terminal::frame_start(), sgr(Role::Reset))?;
        graphics.begin(&mut actual)?;
        let mut painter =
          Painter::new(&mut actual).with_graphics(&mut graphics);
        painter.paint(&retained.scene);
        painter.finish()?;
        graphics.end(&mut actual)?;
        write!(actual, "{}", terminal::frame_end())?;
        assert_eq!(actual, expected, "{view}, focus {focus}");
        assert_eq!(
          legacy_frame(&mut legacy)?,
          expected,
          "legacy {view}, focus {focus}"
        );
        for (left, right) in streamed.tabs[streamed.active]
          .panes
          .iter()
          .zip(&recorded.tabs[recorded.active].panes)
        {
          assert_eq!(left.is_recent(), right.is_recent(), "{view}");
          assert_eq!(
            left.document().map(|app| (
              app.complete,
              app.tiles.wanted(),
              app.outline_pending()
            )),
            right.document().map(|app| (
              app.complete,
              app.tiles.wanted(),
              app.outline_pending()
            )),
            "{view}"
          );
        }
        let tab = &recorded.tabs[recorded.active];
        assert_eq!(
          retained.complete,
          screen.rows != 0
            && tab
              .panes
              .iter()
              .all(|pane| pane.document().is_none_or(|app| app.complete))
        );
        assert_eq!(retained.title, tab.panes[tab.focus].title());
        if view == "prompt" {
          let cursor = retained
            .scene
            .iter()
            .rposition(|op| matches!(op, Op::ShowCursor))
            .expect("prompt cursor");
          assert!(matches!(
            &retained.scene[cursor + 1..],
            [Op::SaveCursor, ..]
          ));
          assert!(matches!(retained.scene.last(), Some(Op::RestoreCursor)));
        }
      }
    }
    Ok(())
  }

  fn legacy_frame(session: &mut Session) -> Result<Vec<u8>> {
    use std::io::Write;
    use vpdf_model::style::Role;
    use vpdf_terminal::{style::sgr, terminal};
    session.prepare_frame();
    let mut out = Vec::new();
    write!(out, "{}{}", terminal::frame_start(), sgr(Role::Reset))?;
    session.graphics.begin(&mut out)?;
    let area = session.area();
    let tab = &mut session.tabs[session.active];
    for index in 0..tab.panes.len() {
      if index != tab.focus {
        legacy_pane(&mut tab.panes[index], &mut session.graphics, &mut out)?;
      }
    }
    crate::presentation::draw(&mut out, None, |scene| {
      tab.tree.scene_dividers(scene, area);
    })?;
    legacy_pane(&mut tab.panes[tab.focus], &mut session.graphics, &mut out)?;
    if let Some(notice) = &session.unmeasured {
      crate::presentation::draw(&mut out, None, |scene| notice.scene(scene))?;
    }
    session.graphics.end(&mut out)?;
    write!(out, "{}", terminal::frame_end())?;
    session.finish_frame();
    Ok(out)
  }

  fn legacy_pane(
    pane: &mut Pane,
    graphics: &mut vpdf_terminal::graphics::Graphics,
    out: &mut Vec<u8>,
  ) -> Result<()> {
    if let Some(app) = pane.document_mut() {
      app.draw_pane(graphics, out)
    } else {
      crate::presentation::draw(out, Some(graphics), |scene| {
        pane.recent_mut().unwrap().scene(scene)
      })?
    }
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn session_frames_preserve_readiness_and_usage() -> Result<()> {
    use crate::jobs::Loading;
    use crate::trace::Drawable;
    let path = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
    let mut streamed = composition_case(&path, "page", 0)?;
    let mut recorded = composition_case(&path, "page", 0)?;
    let mut legacy = composition_case(&path, "page", 0)?;
    let screen = recorded.screen.unwrap();
    let mut cache = Cache::new(BUDGET);
    streamed.draw_frame(&mut Vec::new())?;
    legacy_frame(&mut legacy)?;
    assert!(!frame(&mut recorded, &mut cache, screen)?.complete);
    for session in [&mut streamed, &mut recorded, &mut legacy] {
      session.drawn();
      assert_waiting(session);
    }
    for session in [&mut streamed, &mut legacy] {
      for app in session.tabs[session.active]
        .panes
        .iter_mut()
        .filter_map(Pane::document_mut)
      {
        for key in vpdf_ui::layout::visible(&app.layout, app.reader.scroll) {
          let tile = crate::raster::render(&mut app.doc, key)?;
          session.graphics.keep(app.id, tile);
        }
      }
    }
    for app in recorded.tabs[recorded.active]
      .panes
      .iter_mut()
      .filter_map(Pane::document_mut)
    {
      for key in vpdf_ui::layout::visible(&app.layout, app.reader.scroll) {
        let pixels = vpdf_pdf::raster::raster(&mut app.doc, key)?;
        let weight = pixels.len();
        cache.insert(
          TileId { owner: TileOwner::new(app.id), key },
          Arc::new(pixels),
          weight,
        );
      }
    }
    assert!(frame(&mut recorded, &mut cache, screen)?.complete);
    assert_frame_usage(&mut streamed, &mut legacy)?;
    assert_upload_order(&streamed);
    let resident = streamed.graphics.usage();
    let focused_bytes =
      resident.bytes - streamed.app().unwrap().image_usage.bytes;
    for session in [&mut streamed, &mut recorded, &mut legacy] {
      session.drawn();
      assert!(session.tabs[session.active].panes.iter().all(|app| !matches!(
        app.document().unwrap().outline_load,
        Loading::Deferred
      )));
    }
    assert!(streamed.app().unwrap().image_usage.uploaded > 0);
    streamed.app().unwrap().images_stale = true;
    legacy.app().unwrap().images_stale = true;
    assert_frame_usage(&mut streamed, &mut legacy)?;
    let tab = &streamed.tabs[streamed.active];
    assert_eq!(
      tab.panes[1].document().unwrap().image_usage.bytes,
      resident.bytes
    );
    assert_eq!(
      tab.panes[2].document().unwrap().image_usage.bytes,
      resident.bytes
    );
    assert_eq!(
      tab.panes[0].document().unwrap().image_usage.bytes,
      resident.bytes - focused_bytes
    );
    assert!(!tab.panes[0].document().unwrap().images_stale);
    assert_eq!(streamed.graphics.usage().bytes, resident.bytes - focused_bytes);
    assert_eq!(streamed.graphics.usage().uploaded, resident.uploaded);
    Ok(())
  }

  fn assert_waiting(session: &Session) {
    for app in
      session.tabs[session.active].panes.iter().filter_map(Pane::document)
    {
      assert!(!app.complete);
      assert!(matches!(app.outline_load, crate::jobs::Loading::Deferred));
    }
  }

  fn assert_upload_order(session: &Session) {
    let tab = &session.tabs[session.active];
    let samples = [
      tab.panes[1].document().unwrap().image_usage,
      tab.panes[2].document().unwrap().image_usage,
      tab.panes[0].document().unwrap().image_usage,
      session.graphics.usage(),
    ];
    assert_eq!((samples[0].bytes, samples[0].uploaded), (0, 0));
    for pair in samples.windows(2) {
      assert!(pair[1].bytes > pair[0].bytes);
      assert!(pair[1].uploaded > pair[0].uploaded);
    }
  }

  fn assert_frame_usage(
    streamed: &mut Session,
    legacy: &mut Session,
  ) -> Result<()> {
    use crate::trace::Drawable;
    let mut actual = Vec::new();
    streamed.draw_frame(&mut actual)?;
    assert_eq!(actual, legacy_frame(legacy)?);
    for (left, right) in streamed.tabs[streamed.active]
      .panes
      .iter()
      .zip(&legacy.tabs[legacy.active].panes)
    {
      assert_eq!(
        left.document().unwrap().image_usage.bytes,
        right.document().unwrap().image_usage.bytes
      );
      assert_eq!(
        left.document().unwrap().image_usage.uploaded,
        right.document().unwrap().image_usage.uploaded
      );
      assert_eq!(
        left.document().unwrap().complete,
        right.document().unwrap().complete
      );
    }
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn preview_frames_own_pixels_after_close() -> Result<()> {
    use vpdf_model::document::Target;
    use vpdf_model::text::{PageId, Position};
    let path = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
    let screen = cells(120, 40);
    let mut session = native();
    session.open(&path, true, screen)?;
    session
      .app()
      .unwrap()
      .show_preview(Target::Page(PageId(1)), Position::default())?;
    let mut cache = Cache::new(BUDGET);
    let first = frame(&mut session, &mut cache, screen)?;
    let image = first
      .scene
      .iter()
      .find_map(|op| match op {
        Op::Picture(picture) => Some(picture.image().clone()),
        _ => None,
      })
      .ok_or_else(|| anyhow::anyhow!("Missing native preview picture"))?;
    assert!(image.pixels().iter().any(|byte| *byte != 255));
    session.app().unwrap().overlays.close_preview();
    let closed = frame(&mut session, &mut cache, screen)?;
    assert!(!closed.scene.iter().any(|op| matches!(op, Op::Picture(_))));
    drop(session);
    assert_eq!(Arc::strong_count(image.pixels()), 2);
    drop(first);
    assert_eq!(Arc::strong_count(image.pixels()), 1);
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn pointer_follows_real_links_and_rejects_stale_geometry() -> Result<()> {
    use vpdf_model::document::Target;
    use vpdf_model::input::MouseButton;
    use vpdf_model::text::PageId;
    let path = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
    let mut screen = cells(120, 40);
    let mut session = native();
    session.open(&path, true, screen)?;
    let app = session.app().unwrap();
    let link = app
      .doc
      .links(PageId(1))?
      .iter()
      .find(|link| !matches!(link.target, Target::Uri(_)))
      .cloned()
      .ok_or_else(|| anyhow::anyhow!("Missing corpus citation"))?;
    let source = app.doc.text(PageId(1))?.nearest(
      PageId(1),
      f32::midpoint(link.first().x0, link.first().x1),
      f32::midpoint(link.first().y0, link.first().y1),
    );
    app.go_to(source, true)?;
    let bounds = app.layout.project(PageId(1), link.first(), app.reader.scroll);
    let pixels = (
      f32::midpoint(bounds.x0, bounds.x1) as u16,
      f32::midpoint(bounds.y0, bounds.y1) as u16,
    );
    let mut cache = Cache::new(BUDGET);
    frame(&mut session, &mut cache, screen)?;
    let drawn = screen;
    assert!(apply(
      &mut session,
      mouse(MouseEventKind::Moved, pixels, drawn),
      &mut screen
    ));
    assert_eq!(session.shape(), vpdf_model::style::Shape::Pointer);
    assert!(!apply(
      &mut session,
      mouse(MouseEventKind::Moved, pixels, drawn),
      &mut screen
    ));
    let old_page = session.app().unwrap().reader.cursor.page;
    let stale = Screen { columns: 119, ..screen };
    apply(
      &mut session,
      mouse(MouseEventKind::Down(MouseButton::Left), pixels, stale),
      &mut screen,
    );
    assert!(!session.pointer.holding());
    assert_eq!(session.app().unwrap().reader.cursor.page, old_page);
    for kind in [
      MouseEventKind::Down(MouseButton::Left),
      MouseEventKind::Up(MouseButton::Left),
    ] {
      apply(&mut session, mouse(kind, pixels, drawn), &mut screen);
    }
    let target = match link.target {
      Target::Page(page) | Target::Point { page, .. } => page,
      Target::Uri(_) => unreachable!(),
    };
    assert_eq!(session.app().unwrap().reader.cursor.page, target);
    apply(
      &mut session,
      Input::History { forward: false, pixels, screen: drawn },
      &mut screen,
    );
    assert_eq!(session.app().unwrap().reader.cursor.page, source.page);
    apply(
      &mut session,
      Input::History { forward: true, pixels, screen: drawn },
      &mut screen,
    );
    assert_eq!(session.app().unwrap().reader.cursor.page, target);
    apply(&mut session, Input::Focus(false), &mut screen);
    assert!(!session.pointer.holding());
    assert!(session.pointer.target().is_none());
    Ok(())
  }

  #[test]
  fn home_divider_dispatches_drags() -> Result<()> {
    use vpdf_model::input::MouseButton;
    let (_root, path) = temp_pdf(&["recent"])?;
    let mut screen = cells(120, 40);
    let mut session = native();
    session.state.recent =
      vec![crate::state::tests::entry(&path.to_string_lossy(), 0, 0)];
    session.recent_tab(screen)?;
    session.resize(screen)?;
    let mut cache = Cache::new(BUDGET);
    frame(&mut session, &mut cache, screen)?;
    for (kind, column) in [
      (MouseEventKind::Down(MouseButton::Left), 89),
      (MouseEventKind::Drag(MouseButton::Left), 59),
      (MouseEventKind::Up(MouseButton::Left), 59),
    ] {
      let pixels = (column * screen.cell_width + 1, 10 * screen.cell_height);
      assert!(apply(&mut session, mouse(kind, pixels, screen), &mut screen));
    }
    frame(&mut session, &mut cache, screen)?;
    assert_eq!(
      session.recent_mut().unwrap().view.shelf_mut().unwrap().start(),
      Some(60)
    );
    assert_ne!(
      session.state.recent_layout,
      vpdf_ui::panel::recent::Preferences::default()
    );
    Ok(())
  }

  #[test]
  fn start_screen_click_opens_the_selected_file() -> Result<()> {
    use vpdf_model::input::MouseButton;
    let (_root, path) = temp_pdf(&["recent"])?;
    let mut screen = cells(80, 24);
    let mut session = native();
    session.state.recent =
      vec![crate::state::tests::entry(&path.to_string_lossy(), 0, 0)];
    session.recent_tab(screen)?;
    session.resize(screen)?;
    frame(&mut session, &mut Cache::new(BUDGET), screen)?;
    for kind in [
      MouseEventKind::Down(MouseButton::Left),
      MouseEventKind::Up(MouseButton::Left),
    ] {
      apply(&mut session, mouse(kind, (32, 20), screen), &mut screen);
    }
    assert!(session.focused().is_recent());
    let y = screen.cell_height * 2 + 4;
    apply(
      &mut session,
      mouse(MouseEventKind::Down(MouseButton::Left), (64, y), screen),
      &mut screen,
    );
    assert_eq!(session.tabs.len(), 1);
    assert_eq!(session.app().unwrap().path, std::fs::canonicalize(path)?);
    assert!(session.recent().is_none());
    Ok(())
  }

  #[test]
  fn wheel_routes_to_panels_and_pointer_zoom() -> Result<()> {
    use vpdf_ui::keys::PanelKind;
    let mut screen = cells(80, 24);
    let (_root, _, mut session) = opened(&["first", "second"], screen)?;
    session.app().unwrap().show_panel(
      PanelKind::Version,
      (0..80)
        .map(|row| vpdf_ui::panel::Row::parse(&format!("row {row}")))
        .collect(),
    )?;
    let mut cache = Cache::new(BUDGET);
    frame(&mut session, &mut cache, screen)?;
    let scroll = session.app().unwrap().reader.scroll;
    let down = mouse(MouseEventKind::ScrollDown, (8, 80), screen);
    apply(&mut session, down, &mut screen);
    assert!(session.app().unwrap().panel.as_ref().unwrap().view.top() > 0);
    assert_eq!(session.app().unwrap().reader.scroll, scroll);
    session.app().unwrap().close_panel()?;
    let zoom = session.app().unwrap().layout.zoom();
    let moved = mouse(MouseEventKind::Moved, (320, 100), screen);
    apply(&mut session, moved, &mut screen);
    let mut up = mouse(MouseEventKind::ScrollUp, (320, 100), screen);
    if let Input::Mouse { event, .. } = &mut up {
      event.modifiers = KeyModifiers::CONTROL;
    }
    apply(&mut session, up, &mut screen);
    assert!(session.app().unwrap().layout.zoom() > zoom);
    Ok(())
  }

  fn gesture(
    kind: MouseEventKind,
    pixels: (u16, u16),
    screen: Screen,
    gesture: Gesture,
  ) -> Input {
    let Input::Mouse { event, .. } = mouse(kind, pixels, screen) else {
      unreachable!()
    };
    Input::Gesture { event, gesture, pixels, screen }
  }

  #[test]
  fn trackpad_turns_move_pages_exactly_and_rows_whole() -> Result<()> {
    use vpdf_ui::keys::PanelKind;
    let mut screen = cells(80, 24);
    let pages = ["one", "two", "three", "four"];
    let (_root, _, mut session) = opened(&pages, screen)?;
    let mut cache = Cache::new(BUDGET);
    frame(&mut session, &mut cache, screen)?;
    let rows = f32::from(vpdf_ui::pointer::WHEEL_ROWS);
    let step = rows * f32::from(screen.cell_height);
    let turn = |turns, whole| Gesture::Turn(Precise { turns, whole });
    let down = MouseEventKind::ScrollDown;
    for turns in [0.1, 0.25] {
      let scroll = session.app().unwrap().reader.scroll;
      let input = gesture(down, (320, 100), screen, turn(turns, 0));
      assert!(apply(&mut session, input, &mut screen));
      assert_eq!(session.app().unwrap().reader.scroll, scroll + turns * step);
      assert!(!session.app().unwrap().reader.animation().active());
    }
    session.app().unwrap().show_panel(
      PanelKind::Version,
      (0..80)
        .map(|row| vpdf_ui::panel::Row::parse(&format!("row {row}")))
        .collect(),
    )?;
    frame(&mut session, &mut cache, screen)?;
    let scroll = session.app().unwrap().reader.scroll;
    let top = |session: &mut Session| {
      session.app().unwrap().panel.as_ref().unwrap().view.top()
    };
    let input = gesture(down, (8, 80), screen, turn(0.4, 0));
    apply(&mut session, input, &mut screen);
    assert_eq!(top(&mut session), 0);
    let input = gesture(down, (8, 80), screen, turn(0.4, 1));
    apply(&mut session, input, &mut screen);
    assert_eq!(top(&mut session), usize::from(vpdf_ui::pointer::WHEEL_ROWS));
    // Pinches over a panel leave the page.
    let zoom = session.app().unwrap().layout.zoom();
    let input = gesture(down, (8, 80), screen, Gesture::Pinch(1.5));
    assert!(!apply(&mut session, input, &mut screen));
    assert_eq!(session.app().unwrap().layout.zoom(), zoom);
    assert_eq!(session.app().unwrap().reader.scroll, scroll);
    Ok(())
  }

  #[test]
  fn pinches_keep_the_page_under_the_pointer() -> Result<()> {
    let mut screen = cells(80, 24);
    let pages = ["one", "two", "three", "four"];
    let (_root, _, mut session) = opened(&pages, screen)?;
    let mut cache = Cache::new(BUDGET);
    frame(&mut session, &mut cache, screen)?;
    let pixels = (320, 100);
    let spot = (pixels.0 / screen.cell_width, pixels.1 / screen.cell_height);
    let pixel = Some(vpdf_ui::pointer::Pixel {
      x: pixels.0,
      y: pixels.1,
      cell: screen.cell(),
    });
    let under = |session: &mut Session| {
      session
        .app()
        .unwrap()
        .page_point(spot.0, spot.1, pixel)
        .expect("page point")
    };
    let start = (session.app().unwrap().layout.zoom(), under(&mut session));
    let kind = MouseEventKind::ScrollUp;
    for factor in [1.25, 1.25, 1.25, 0.8, 0.8, 0.8] {
      let before = under(&mut session);
      let zoom = session.app().unwrap().layout.zoom();
      let input = gesture(kind, pixels, screen, Gesture::Pinch(factor));
      assert!(apply(&mut session, input, &mut screen));
      assert!(
        (session.app().unwrap().layout.zoom() - zoom * factor).abs() < 1e-4
      );
      let after = under(&mut session);
      assert_eq!(after.0, before.0);
      assert!((after.1 - before.1).abs() < 0.5, "{before:?} {after:?}");
      assert!((after.2 - before.2).abs() < 0.5, "{before:?} {after:?}");
    }
    let end = (session.app().unwrap().layout.zoom(), under(&mut session));
    assert!((end.0 - start.0).abs() < 1e-4);
    assert_eq!(end.1.0, start.1.0);
    assert!((end.1.2 - start.1.2).abs() < 0.5, "{start:?} {end:?}");
    // Zoom stays finite and bounded.
    for factor in [f32::INFINITY, 1e9, 1e-9] {
      let input = gesture(kind, pixels, screen, Gesture::Pinch(factor));
      apply(&mut session, input, &mut screen);
      let zoom = f64::from(session.app().unwrap().layout.zoom());
      assert!(
        (vpdf_ui::layout::MIN_ZOOM..=vpdf_ui::layout::MAX_ZOOM).contains(&zoom)
      );
    }
    Ok(())
  }

  #[test]
  fn focus_loss_releases_active_pointer_capture() -> Result<()> {
    use vpdf_model::input::MouseButton;
    let mut screen = cells(80, 24);
    let (_root, _, mut session) = opened(&["first", "second"], screen)?;
    let down =
      mouse(MouseEventKind::Down(MouseButton::Left), (80, 128), screen);
    apply(&mut session, down, &mut screen);
    assert!(session.pointer.holding());
    apply(&mut session, Input::Focus(false), &mut screen);
    assert!(!session.pointer.holding());
    assert_eq!(session.sight, super::super::Sight::Unfocused);
    apply(&mut session, Input::Focus(true), &mut screen);
    assert_eq!(session.sight, super::super::Sight::Focused);
    Ok(())
  }

  #[test]
  fn clipboard_frames_drain_panes() -> Result<()> {
    use vpdf_reader::reader::Yank;
    let root = tempfile::tempdir()?;
    let screen = cells(80, 24);
    let mut session = native();
    for name in ["first.pdf", "second.pdf"] {
      let path = root.path().join(name);
      crate::fixtures::pdf(&path, &["body"], "")?;
      session.open(&path, true, screen)?;
    }
    let mut cache = Cache::new(BUDGET);
    for (register, expected) in [('a', "first"), ('"', "second")] {
      session.tabs[0].panes[0]
        .document_mut()
        .unwrap()
        .reader
        .deliver_yank(Yank { text: "first".into(), register: '"' });
      session
        .app()
        .unwrap()
        .reader
        .deliver_yank(Yank { text: "second".into(), register });
      assert_eq!(
        frame(&mut session, &mut cache, screen)?.clipboard.as_deref(),
        Some(expected)
      );
      assert!(frame(&mut session, &mut cache, screen)?.clipboard.is_none());
    }
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn hidden_region_jobs_finish() -> Result<()> {
    use vpdf_model::text::{Bounds, PageId};
    use vpdf_reader::reader::Yank;
    use vpdf_ui::region::Operation;
    let source = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
    let root = tempfile::tempdir()?;
    let first = root.path().join("first.pdf");
    let second = root.path().join("second.pdf");
    for path in [&first, &second] {
      std::fs::copy(&source, path)?;
    }
    let screen = cells(120, 40);
    let (wake, wakes) = crate::fixtures::wake();
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(wake);
    session.open(&first, true, screen)?;
    session.app().unwrap().select_region(
      PageId(0),
      Bounds { x0: 10.0, y0: 10.0, x1: 50.0, y1: 50.0 },
    )?;
    session.app().unwrap().region_operation(Operation::Save)?;
    session
      .app()
      .unwrap()
      .reader
      .deliver_yank(Yank { text: "pending region text".into(), register: '"' });
    session.open(&second, true, screen)?;
    let mut cache = Cache::new(BUDGET);
    let deadline = Instant::now() + Duration::from_secs(10);
    while session.tabs[0].panes[0].document().unwrap().region_job.is_some() {
      wakes.recv_timeout(deadline.saturating_duration_since(Instant::now()))?;
      completions(&mut session, &mut cache, &mut Vec::new());
    }
    assert!(
      session.tabs[0].panes[0]
        .document()
        .unwrap()
        .reader
        .message
        .starts_with("Saved ")
    );
    assert!(
      std::fs::read(root.path().join("first-p1-region-1.png"))?
        .starts_with(b"\x89PNG\r\n\x1a\n")
    );
    assert_eq!(
      frame(&mut session, &mut cache, screen)?.clipboard.as_deref(),
      Some("pending region text")
    );
    Ok(())
  }

  #[test]
  fn opens_preserve_split_tabs() -> Result<()> {
    let root = tempfile::tempdir()?;
    let first = root.path().join("first.pdf");
    let second = root.path().join("second.pdf");
    for path in [&first, &second] {
      crate::fixtures::pdf(path, &["body"], "")?;
    }
    let mut screen = cells(80, 24);
    let mut session = native();
    ensure_home(&mut session, screen);
    apply(&mut session, Input::Resize(screen), &mut screen);
    apply(&mut session, key(KeyCode::Char(':')), &mut screen);
    assert!(session.recent().unwrap().view.typing());
    apply(&mut session, Input::Open(first.clone()), &mut screen);
    assert!(session.recent().is_none());
    session.app().unwrap().command("vsplit")?;
    session.settle()?;
    let owners: Vec<_> = session.tab().panes.iter().map(|app| app.id).collect();
    apply(&mut session, key(KeyCode::Char(':')), &mut screen);
    assert!(session.app().unwrap().overlays.prompt().is_some());
    apply(&mut session, Input::Open(second.clone()), &mut screen);
    assert_eq!(session.tabs.len(), 2);
    assert_eq!(session.active, 1);
    assert_eq!(session.app().unwrap().path, std::fs::canonicalize(second)?);
    assert_eq!(
      session.tabs[0].panes.iter().map(|app| app.id).collect::<Vec<_>>(),
      owners
    );
    apply(&mut session, Input::Open(first), &mut screen);
    assert_eq!(session.tabs.len(), 2);
    assert_eq!(session.active, 0);
    Ok(())
  }

  #[test]
  fn committed_text_uses_shared_prompt_and_undo() -> Result<()> {
    let mut screen = cells(80, 24);
    let (_root, _, mut session) = opened(&["body"], screen)?;
    apply(&mut session, key(KeyCode::Char('/')), &mut screen);
    apply(&mut session, Input::Paste("章a\u{301}".into()), &mut screen);
    assert_eq!(
      session.app().unwrap().overlays.prompt().unwrap().text(),
      "章a\u{301}"
    );
    apply(
      &mut session,
      Input::Key(KeyEvent::new(KeyCode::Char('_'), KeyModifiers::CONTROL)),
      &mut screen,
    );
    assert_eq!(session.app().unwrap().overlays.prompt().unwrap().text(), "");
    Ok(())
  }

  #[test]
  fn recolor_prompt_discards_old_pixels() -> Result<()> {
    use vpdf_pdf::jobs::render::Rendered;
    let mut screen = crate::screen::pixels(60, 20, 10, 20);
    let (_root, _, mut session) = opened(&["body"], screen)?;
    let old = vpdf_ui::layout::visible(&session.app().unwrap().layout, 0.0)[0];
    apply(&mut session, key(KeyCode::Char(':')), &mut screen);
    apply(&mut session, Input::Paste("recolor dark 60".into()), &mut screen);
    apply(&mut session, key(KeyCode::Enter), &mut screen);
    let app = session.app().unwrap();
    let recolor = vpdf_model::recolor::Recolor::parse("dark 60").unwrap();
    assert_eq!(app.layout.recolor(), recolor);
    let new = vpdf_model::tile::TileKey { recolor, ..old };
    let owner = TileOwner::new(app.id);
    let (send, receive) = mpsc::sync_channel(2);
    for key in [old, new] {
      send.send(Ok(Rendered { key, pixels: vec![0; 3], render_ms: 0.0 }))?;
    }
    app.worker.raw = Some(crate::raster::Raw {
      results: receive,
      overflow: Arc::new(AtomicBool::new(false)),
    });
    let mut cache = Cache::new(BUDGET);
    completions(&mut session, &mut cache, &mut Vec::new());
    assert!(!cache.contains(&TileId { owner, key: old }));
    assert!(cache.contains(&TileId { owner, key: new }));
    Ok(())
  }

  #[test]
  fn overflow_restarts_delivery_and_keeps_received_pixels() -> Result<()> {
    use vpdf_pdf::jobs::render::Rendered;
    let screen = crate::screen::pixels(60, 20, 10, 20);
    let (_root, _, mut session) = opened(&["body"], screen)?;
    let app = session.app().unwrap();
    let key = vpdf_ui::layout::visible(&app.layout, app.reader.scroll)[0];
    let id = TileId { owner: TileOwner::new(app.id), key };
    let (send, receive) = mpsc::sync_channel(1);
    send.send(Ok(Rendered {
      key,
      pixels: vec![255; key.width as usize * key.height as usize * 3],
      render_ms: 0.0,
    }))?;
    app.worker.raw = Some(crate::raster::Raw {
      results: receive,
      overflow: Arc::new(AtomicBool::new(true)),
    });
    let mut cache = Cache::new(BUDGET);
    assert!(completions(&mut session, &mut cache, &mut Vec::new()));
    assert!(cache.contains(&id));
    assert!(send.try_send(Err("old receiver".into())).is_err());
    assert!(
      !session
        .app()
        .unwrap()
        .worker
        .raw
        .as_ref()
        .unwrap()
        .overflow
        .load(Ordering::Acquire)
    );
    Ok(())
  }

  #[test]
  fn home_reports_open_failure() -> Result<()> {
    let (mut bridge, wakes) = launch(Vec::new(), None, None)?;
    wait(&bridge, &wakes, |_| true)?;
    bridge.send(Input::Open(PathBuf::from("/missing/native.pdf")));
    let frame = wait(&bridge, &wakes, |frame| {
      frame
        .scene
        .iter()
        .any(|op| matches!(op, Op::Style(vpdf_model::style::Role::Error)))
    })?;
    assert_eq!(frame.owners, [] as [TileOwner; 0]);
    bridge.close()
  }

  #[test]
  fn home_removal_returns_to_welcome() -> Result<()> {
    let (root, pdf) = temp_pdf(&["body"])?;
    let mut session = native();
    session.store = Some(root.path().join("state.json"));
    session.state.opened(crate::state::tests::entry(
      &pdf.to_string_lossy(),
      0,
      0,
    ));
    let mut screen = cells(120, 40);
    session.recent_tab(screen)?;
    assert!(apply(&mut session, key(KeyCode::Char(' ')), &mut screen));
    assert!(apply(&mut session, key(KeyCode::Char('x')), &mut screen));
    assert!(!apply(&mut session, Input::Open(pdf.clone()), &mut screen));
    assert!(session.focused().is_recent());
    assert!(apply(&mut session, Input::Paste(":q".into()), &mut screen));
    assert!(apply(&mut session, key(KeyCode::Enter), &mut screen));
    assert_eq!(session.state.recent, []);
    assert_eq!(session.history.commands(), [] as [String; 0]);
    assert!(pdf.exists());
    let home = session.recent_mut().unwrap();
    let mut scene = Recorder::new(|_| false);
    home.scene(&mut scene)?;
    let scene = scene.into_scene();
    assert!(shows(&scene, ":edit <file>"));
    assert!(!shows(&scene, "Remove "));
    Ok(())
  }
  #[test]
  fn references_keep_per_document_targets() -> Result<()> {
    use vpdf_model::reference::Reference;
    let root = tempfile::tempdir()?;
    let first = root.path().join("first.pdf");
    let second = root.path().join("second.pdf");
    crate::fixtures::pdf(&first, &["one", "two", "three"], "/Rotate 90")?;
    crate::fixtures::pdf(&second, &["one", "two", "three"], "/Rotate 90")?;
    let first = std::fs::canonicalize(first)?;
    let second = std::fs::canonicalize(second)?;
    let parse = |path: &std::path::Path, target: &str| -> Result<Reference> {
      Ok(Reference::parse(format!("{}{target}", path.display()).as_ref())?)
    };
    let store = root.path().join("state");
    let (wake, wakes) = crate::fixtures::wake();
    let mut bridge = Bridge::configured(
      Launch {
        references: vec![
          parse(&first, "#search=%22two%22&page=3")?,
          parse(&second, "#page=2")?,
        ],
        store: Some(store.clone()),
        config: crate::config::Loaded::default(),
        ..Launch::default()
      },
      move || wake.wake(),
    )?;
    bridge.send(Input::Observe(true));
    wait(&bridge, &wakes, |frame| {
      frame.metrics.observation.document.as_ref() == Some(&first)
        && frame.metrics.page == Some(vpdf_model::text::PageId(2))
        && !frame.metrics.searching
    })
    .map_err(|error| anyhow::anyhow!("First reference: {error}"))?;
    bridge.send(Input::Reference(parse(&second, "")?));
    wait(&bridge, &wakes, |frame| {
      frame.metrics.observation.document.as_ref() == Some(&second)
        && frame.metrics.page == Some(vpdf_model::text::PageId(1))
    })?;
    bridge.send(Input::Reference(parse(&first, "#page=1")?));
    wait(&bridge, &wakes, |frame| {
      frame.metrics.observation.document.as_ref() == Some(&first)
        && frame.metrics.page == Some(vpdf_model::text::PageId(0))
    })?;
    bridge.close()?;
    let state = crate::state::State::load(&store);
    assert_eq!(state.entry(&first).unwrap().cursor.page.0, 0);
    assert_eq!(state.entry(&second).unwrap().cursor.page.0, 1);
    Ok(())
  }
  #[test]
  fn entrypoints_share_object_targets() -> Result<()> {
    use vpdf_model::reference::Reference;
    let file = crate::reference_fixture::pdf(90)?;
    let reference = Reference::parse(
      format!(
        "{}{}",
        file.path().display(),
        concat!(
          "#page=1&comment=note&nameddest=proof&structelem=%FF",
          "&search=%22two%22&page=1&highlight=5,25,10,30"
        )
      )
      .as_ref(),
    )?;
    let mut terminal = Session::new(None);
    terminal.open_reference(&reference, true, cells(100, 35))?;
    super::super::reference_tests::finish_search(terminal.app().unwrap())?;
    assert_eq!(terminal.app().unwrap().reader.cursor.page.0, 0);
    assert_eq!(
      terminal.app().unwrap().reference.rectangle.as_ref().unwrap().page.0,
      0
    );
    let (wake, wakes) = crate::fixtures::wake();
    let mut bridge = Bridge::configured(
      Launch {
        references: vec![reference],
        store: None,
        config: crate::config::Loaded::default(),
        ..Launch::default()
      },
      move || wake.wake(),
    )?;
    bridge.send(Input::Observe(true));
    wait(&bridge, &wakes, |frame| {
      frame.metrics.observation.hits == 1
        && frame.metrics.observation.error.is_none()
        && !frame.metrics.searching
        && frame.metrics.page == Some(vpdf_model::text::PageId(0))
    })?;
    for (target, expected) in [
      ("#page=99", "Page outside document"),
      ("#nameddest=missing", "Named destination missing"),
      ("#structelem=missing", "Structure ID missing"),
      ("#comment=missing", "Annotation name missing"),
    ] {
      bridge.send(Input::Reference(Reference::parse(
        format!("{}{target}", file.path().display()).as_ref(),
      )?));
      wait(&bridge, &wakes, |frame| {
        frame
          .metrics
          .observation
          .error
          .as_deref()
          .is_some_and(|error| error.contains(expected))
      })?;
    }
    bridge.close()
  }
  #[test]
  fn reference_errors_survive_startup_warning() -> Result<()> {
    use vpdf_model::reference::Reference;
    let (_root, path) = temp_pdf(&["one"])?;
    let warning = "Reference configuration warning";
    for (input, expected) in [
      (format!("{}#page=99", path.display()), "Page outside document"),
      (format!("{}-missing.pdf#page=2", path.display()), "No such file"),
    ] {
      let config = crate::config::Loaded {
        warning: Some(warning.into()),
        ..crate::config::Loaded::default()
      };
      let (wake, wakes) = crate::fixtures::wake();
      let mut bridge = Bridge::configured(
        Launch {
          references: vec![Reference::parse(input.as_ref())?],
          store: None,
          config,
          ..Launch::default()
        },
        move || wake.wake(),
      )?;
      let failed =
        wait(&bridge, &wakes, |frame| shows(&frame.scene, expected))?;
      assert!(!shows(&failed.scene, warning));
      bridge.send(Input::Reference(Reference::parse(
        format!("{}#page=1", path.display()).as_ref(),
      )?));
      wait(&bridge, &wakes, |frame| shows(&frame.scene, warning))?;
      bridge.close()?;
    }
    Ok(())
  }
  #[test]
  fn native_commands_reject_stale_targets() -> Result<()> {
    use commands::Action;
    let (_root, path, mut session) = opened(&["one"], sized())?;
    let (_other, picked) = temp_pdf(&["two"])?;
    let mut screen = sized();
    commands::refresh(&mut session);
    let command = session.native.snapshot.command(Action::Open).unwrap();
    let old = Arc::clone(&session.native.snapshot);
    commands::refresh(&mut session);
    assert!(Arc::ptr_eq(&old, &session.native.snapshot));
    session.app().unwrap().document_generation += 1;
    assert!(!apply(&mut session, Input::Action(command), &mut screen));
    assert_eq!(session.native.effects, []);
    let command = session.native.snapshot.command(Action::Open).unwrap();
    assert!(apply(&mut session, Input::Action(command), &mut screen));
    assert!(!apply(&mut session, Input::Action(command), &mut screen));
    let request = requested(&session);
    assert_eq!(session.native.effects.len(), 1);
    session.app().unwrap().document_generation += 1;
    assert!(apply(
      &mut session,
      Input::Picked { request, paths: vec![picked] },
      &mut screen,
    ));
    assert_eq!(session.tabs.len(), 1);
    assert_eq!(
      session.app().unwrap().document_identity(),
      path.canonicalize()?
    );
    assert!(session.native.snapshot.command(Action::Open).is_some());
    Ok(())
  }

  #[test]
  fn native_commands_track_focus_epochs() -> Result<()> {
    use commands::Action;
    let (_root, _path, mut session) = opened(&["one"], sized())?;
    let (_other, path) = temp_pdf(&["two"])?;
    let mut screen = sized();
    commands::refresh(&mut session);
    let old = session.native.snapshot.command(Action::Open).unwrap();
    session.open(&path, true, screen)?;
    session.native.effects.clear();
    commands::refresh(&mut session);
    session.activate(0)?;
    assert!(!apply(&mut session, Input::Action(old), &mut screen));
    assert_eq!(session.native.effects, []);
    Ok(())
  }

  fn requested(session: &Session) -> commands::RequestId {
    session
      .native
      .effects
      .iter()
      .rev()
      .find_map(|effect| match effect {
        commands::Effect::Open { request } => Some(*request),
        _ => None,
      })
      .expect("Open request")
  }

  // Mirrors the run loop's one-per-turn queue.
  fn open_queued(session: &mut Session, screen: &mut Screen) {
    while let Some(path) = session.native_opens.pop_front() {
      apply(session, Input::Reference(path), screen);
    }
  }

  #[test]
  fn native_panel_completion_is_once() -> Result<()> {
    use commands::Action;
    let (_root, path) = temp_pdf(&["one"])?;
    let mut session = native();
    let mut screen = sized();
    ensure_home(&mut session, screen);
    commands::refresh(&mut session);
    let command = session.native.snapshot.command(Action::Open).unwrap();
    apply(&mut session, Input::Action(command), &mut screen);
    let request = requested(&session);
    let source = std::fs::read(&path)?;
    assert!(apply(
      &mut session,
      Input::Picked { request, paths: vec![path.clone()] },
      &mut screen
    ));
    assert!(!apply(
      &mut session,
      Input::Picked { request, paths: vec![path.clone()] },
      &mut screen
    ));
    open_queued(&mut session, &mut screen);
    assert_eq!(session.tabs.len(), 1);
    assert_eq!(std::fs::read(path)?, source);
    let command = session.native.snapshot.command(Action::Open).unwrap();
    apply(&mut session, Input::Action(command), &mut screen);
    let request = requested(&session);
    assert!(apply(
      &mut session,
      Input::Picked { request, paths: Vec::new() },
      &mut screen
    ));
    assert!(session.native.snapshot.command(Action::Open).is_some());
    assert_eq!(session.tabs.len(), 1);
    Ok(())
  }

  #[test]
  fn native_panel_queues_many_files() -> Result<()> {
    use commands::{Action, Effect};
    let (_first_root, first) = temp_pdf(&["first"])?;
    let (_other, unicode) = temp_pdf(&["third"])?;
    let renamed = unicode.with_file_name("Résumé 文書.pdf");
    std::fs::rename(&unicode, &renamed)?;
    let mut session = native();
    let mut screen = sized();
    ensure_home(&mut session, screen);
    commands::refresh(&mut session);
    let command = session.native.snapshot.command(Action::Open).unwrap();
    apply(&mut session, Input::Action(command), &mut screen);
    let request = requested(&session);
    let paths =
      vec![first.clone(), PathBuf::from("/missing.pdf"), renamed.clone()];
    assert!(apply(&mut session, Input::Picked { request, paths }, &mut screen));
    assert!(session.focused().is_recent());
    assert_eq!(session.native_opens.len(), 3);
    open_queued(&mut session, &mut screen);
    assert_eq!(session.tabs.len(), 2);
    apply(&mut session, Input::Open(first.clone()), &mut screen);
    assert_eq!(session.tabs.len(), 2);
    let recent: Vec<_> = session
      .native
      .effects
      .iter()
      .filter_map(|effect| match effect {
        Effect::Recent(path) => Some(path.clone()),
        _ => None,
      })
      .collect();
    let first = first.canonicalize()?;
    assert_eq!(recent, [first.clone(), renamed.canonicalize()?, first.clone()]);
    assert_eq!(session.native.snapshot.path.as_deref(), Some(first.as_path()));
    Ok(())
  }

  #[test]
  fn native_reveal_targets_documents() -> Result<()> {
    use commands::{Action, Effect};
    let (_root, path, mut session) = opened(&["one"], sized())?;
    let mut screen = sized();
    commands::refresh(&mut session);
    let command = session.native.snapshot.command(Action::Reveal).unwrap();
    assert!(apply(&mut session, Input::Action(command), &mut screen));
    assert_eq!(
      session.native.effects.last(),
      Some(&Effect::Reveal(path.canonicalize()?))
    );
    session.request_help("", vpdf_ui::topics::Invocation::Home, screen)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while session.app().unwrap().manual.is_none() {
      session.help_updates(&mut std::io::sink());
      assert!(Instant::now() < deadline, "Help generation timed out");
      thread::sleep(Duration::from_millis(2));
    }
    commands::refresh(&mut session);
    assert!(session.native.snapshot.command(Action::Reveal).is_none());
    assert!(
      session.native.snapshot.path.as_ref().is_some_and(|p| !p.is_absolute())
    );
    assert!(matches!(session.native.effects.last(), Some(Effect::Reveal(_))));
    Ok(())
  }

  #[test]
  fn terminal_opens_skip_system_recents() -> Result<()> {
    let (_root, path) = temp_pdf(&["one"])?;
    let mut session = Session::new(None);
    session.open(&path, true, sized())?;
    assert_eq!(session.native.effects, []);
    Ok(())
  }

  #[test]
  fn native_effects_survive_frame_replacement() -> Result<()> {
    use commands::Action;
    let (_root, _path, mut session) = opened(&["one"], sized())?;
    let mut screen = sized();
    commands::refresh(&mut session);
    let command = session.native.snapshot.command(Action::Open).unwrap();
    apply(&mut session, Input::Action(command), &mut screen);
    let mut cache = Cache::new(BUDGET);
    let latest = Mutex::new(None);
    publish(&latest, frame(&mut session, &mut cache, screen)?);
    publish(&latest, frame(&mut session, &mut cache, screen)?);
    assert_eq!(latest.lock().unwrap().take().unwrap().effects.len(), 1);
    publish(&latest, frame(&mut session, &mut cache, screen)?);
    assert_eq!(latest.lock().unwrap().take().unwrap().effects, []);
    Ok(())
  }
  fn command(
    session: &mut Session,
    action: commands::Action,
    screen: &mut Screen,
  ) {
    commands::refresh(session);
    let command =
      session.native.snapshot.command(action).expect("Command available");
    assert!(apply(session, Input::Action(command), screen));
  }

  #[test]
  fn mac_commands_close_tabs_and_splits() -> Result<()> {
    use commands::Action;
    let (_root, _path, mut session) = opened(&["one"], sized())?;
    let mut screen = sized();
    session.split(
      vpdf_ui::split::Axis::Columns,
      None,
      vpdf_ui::split::Place::After,
    )?;
    command(&mut session, Action::NewTab, &mut screen);
    assert_eq!(session.tabs.len(), 2);
    command(&mut session, Action::PreviousTab, &mut screen);
    assert_eq!(session.tabs[session.active].panes.len(), 2);
    command(&mut session, Action::CloseTab, &mut screen);
    assert_eq!(session.tabs.len(), 1);
    command(&mut session, Action::CloseTab, &mut screen);
    assert!(session.tabs.is_empty());
    assert!(session.recent().is_none());
    assert!(session.quit);
    Ok(())
  }

  #[test]
  fn mac_prompt_clipboard_is_selection_aware() -> Result<()> {
    use commands::Action;
    let (_root, _path, mut session) = opened(&["one"], sized())?;
    let mut screen = sized();
    apply(&mut session, key(KeyCode::Char(':')), &mut screen);
    apply(&mut session, Input::Paste("alpha beta".into()), &mut screen);
    command(&mut session, Action::SelectAll, &mut screen);
    session.app().unwrap().reader.store('"', "older".into())?;
    command(&mut session, Action::Copy, &mut screen);
    assert_eq!(take_clipboard(&mut session).as_deref(), Some("alpha beta"));
    assert!(take_clipboard(&mut session).is_none());
    command(&mut session, Action::Cut, &mut screen);
    assert_eq!(take_clipboard(&mut session).as_deref(), Some("alpha beta"));
    assert_eq!(session.app().unwrap().overlays.prompt().unwrap().text(), "");
    command(&mut session, Action::Undo, &mut screen);
    assert_eq!(
      session.app().unwrap().overlays.prompt().unwrap().text(),
      "alpha beta"
    );
    let old = session.native.snapshot.command(Action::Paste).unwrap();
    apply(&mut session, key(KeyCode::Esc), &mut screen);
    assert!(!apply(
      &mut session,
      Input::PasteTarget { command: old, text: "lost".into() },
      &mut screen
    ));
    assert!(session.app().unwrap().overlays.prompt().is_none());
    Ok(())
  }

  #[test]
  fn mac_document_copy_yields_and_cancels() -> Result<()> {
    use commands::Action;
    let (_root, path, mut session) =
      opened(&["first", "second", "last"], sized())?;
    let hash = std::fs::read(&path)?;
    let mut screen = sized();
    let location = session.app().unwrap().navigation_position();
    let cached = session.app().unwrap().doc.cache_bytes();
    command(&mut session, Action::SelectAll, &mut screen);
    assert_eq!(session.app().unwrap().navigation_position(), location);
    assert_eq!(session.app().unwrap().doc.cache_bytes(), cached);
    assert!(session.app().unwrap().reader.document_selected);
    command(&mut session, Action::Copy, &mut screen);
    assert!(commands::copy_update(&mut session));
    assert!(take_clipboard(&mut session).is_none());
    assert!(commands::copying(&session));
    assert!(commands::copy_update(&mut session));
    assert!(commands::copy_update(&mut session));
    assert_eq!(
      take_clipboard(&mut session).as_deref(),
      Some("first\nsecond\nlast\n")
    );
    command(&mut session, Action::Copy, &mut screen);
    commands::copy_update(&mut session);
    apply(&mut session, key(KeyCode::Esc), &mut screen);
    assert!(!commands::copying(&session));
    assert!(!session.app().unwrap().reader.document_selected);
    assert!(take_clipboard(&mut session).is_none());
    assert_eq!(std::fs::read(path)?, hash);
    Ok(())
  }
  #[test]
  fn mac_find_preserves_query_and_location() -> Result<()> {
    use commands::Action;
    let (_root, _path, mut session) = opened(&["one two one"], sized())?;
    let mut screen = sized();
    let location = session.app().unwrap().navigation_position();
    command(&mut session, Action::Find, &mut screen);
    apply(&mut session, Input::Paste("one".into()), &mut screen);
    command(&mut session, Action::Find, &mut screen);
    assert_eq!(
      session.app().unwrap().overlays.prompt().unwrap().selected(),
      Some("one")
    );
    command(&mut session, Action::NextResult, &mut screen);
    assert_eq!(session.app().unwrap().overlays.prompt().unwrap().text(), "one");
    apply(&mut session, key(KeyCode::Esc), &mut screen);
    assert_eq!(session.app().unwrap().navigation_position(), location);
    Ok(())
  }

  #[test]
  fn mac_toolbar_toggles_sidebar_and_pages() -> Result<()> {
    use commands::Action;
    let (_root, _path, mut session) =
      opened(&["first", "second", "last"], sized())?;
    let mut screen = sized();
    commands::refresh(&mut session);
    assert_eq!(session.native.snapshot.pages, 3);
    assert_eq!(session.native.snapshot.panel, None);
    command(&mut session, Action::Sidebar, &mut screen);
    assert_eq!(
      session.native.snapshot.panel,
      Some(vpdf_ui::keys::PanelKind::Outline)
    );
    assert_eq!(
      session.native.snapshot.focus,
      commands::Focus::Reader(vpdf_ui::input::InputOwner::Content)
    );
    command(&mut session, Action::Sidebar, &mut screen);
    assert_eq!(session.native.snapshot.panel, None);
    let go = session.native.snapshot.command(Action::GoToPage).unwrap();
    for (page, expected) in [(3, 2), (9, 2), (0, 0)] {
      assert!(apply(
        &mut session,
        Input::Page { command: go, page },
        &mut screen
      ));
      assert_eq!(session.app().unwrap().reader.cursor.page.0, expected);
    }
    let paste = Input::Page {
      command: commands::Command { action: Action::Paste, ..go },
      page: 2,
    };
    assert!(!apply(&mut session, paste, &mut screen));
    session.app().unwrap().document_generation += 1;
    assert!(!apply(
      &mut session,
      Input::Page { command: go, page: 2 },
      &mut screen
    ));
    assert_eq!(session.app().unwrap().reader.cursor.page.0, 0);
    Ok(())
  }

  #[test]
  fn mac_settings_opens_config_file() -> Result<()> {
    use commands::{Action, Effect};
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("vpdf/config.toml");
    let source =
      crate::config::Source::File { path: path.clone(), explicit: false };
    let config = crate::config::Loaded { source, ..Default::default() };
    let mut session = Session::configured(None, config);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
    let mut screen = sized();
    ensure_home(&mut session, screen);
    command(&mut session, Action::Settings, &mut screen);
    assert_eq!(std::fs::read(&path)?, b"");
    let opened = Effect::EditConfig(path.clone());
    assert_eq!(session.native.effects.last(), Some(&opened));
    std::fs::write(&path, "[reading]\nzoom = 2.0\n")?;
    command(&mut session, Action::Settings, &mut screen);
    assert_eq!(std::fs::read_to_string(&path)?, "[reading]\nzoom = 2.0\n");
    let mut disabled = native();
    ensure_home(&mut disabled, screen);
    commands::refresh(&mut disabled);
    assert!(disabled.native.snapshot.command(Action::Settings).is_none());
    Ok(())
  }
}
