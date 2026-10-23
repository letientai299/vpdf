//! Session ownership for native windows.
use super::interaction::Interaction;
use super::maintenance::Chores;
use super::unlock::Operation;
use super::{Home, Screen, Session};
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

const BUDGET: usize = 64 << 20;
/// Inputs one frame takes beyond the first, so a stream cannot hold it.
const BATCH: usize = 64;

pub enum Input {
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
  Key(KeyEvent),
  Paste(String),
  Mouse {
    event: MouseEvent,
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
  Open(PathBuf),
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
  pub metrics: Metrics,
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
  send: Mutex<mpsc::Sender<(Ticket, Input)>>,
  next: AtomicU64,
  latest: Arc<Mutex<Option<Frame>>>,
  thread: Option<JoinHandle<Result<()>>>,
}

struct Launch {
  config: crate::config::Loaded,
  paths: Vec<PathBuf>,
  store: Option<PathBuf>,
  page: Option<usize>,
}

impl Bridge {
  pub fn configured(
    paths: Vec<PathBuf>,
    store: Option<PathBuf>,
    page: Option<usize>,
    config: crate::config::Loaded,
    repaint: impl Fn() + Send + Sync + 'static,
  ) -> Result<Self> {
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
          Launch { config, paths, store, page },
          &receive,
          &output,
          &repaint,
          &wake,
          &pending,
        )
      })?;
    Ok(Self {
      send: Mutex::new(send),
      next: AtomicU64::new(1),
      latest,
      thread: Some(worker),
    })
  }

  pub fn send(&self, input: Input) -> Ticket {
    let sender =
      self.send.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let ticket = Ticket(self.next.fetch_add(1, Ordering::Relaxed));
    let _ = sender.send((ticket, input));
    ticket
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

fn run(
  launch: Launch,
  receive: &mpsc::Receiver<(Ticket, Input)>,
  latest: &Mutex<Option<Frame>>,
  repaint: &impl Fn(),
  wake: &Wake,
  pending: &AtomicBool,
) -> Result<()> {
  let mut starting = !launch.paths.is_empty();
  let mut session = Session::configured(launch.store, launch.config);
  session.frontend = crate::raster::Frontend::Native(wake.clone());
  let mut screen = Screen {
    columns: 100,
    rows: 40,
    cell_width: 10,
    cell_height: 20,
    origin: (0, 0),
  };
  ensure_home(&mut session, screen);
  for path in launch.paths {
    apply(&mut session, Input::Open(path), &mut screen);
  }
  session.resize(screen)?;
  let mut cache = Cache::new(BUDGET);
  let mut reloaded = Vec::new();
  let mut chores = Chores::new(Instant::now());
  let mut dirty = true;
  let mut animation_due = Instant::now();
  let mut applied = Ticket::default();
  loop {
    dirty |= session.help_updates(&mut std::io::sink());
    dirty |= session.print_update();
    dirty |= picker_update(&mut session, screen);
    dirty |= resume_keys(&mut session);
    if !session.blocked()
      && session.pending_keys.is_empty()
      && let Some(path) = session.native_opens.pop_front()
    {
      apply(&mut session, Input::Open(path), &mut screen);
      dirty = true;
    }
    finish_launch(&mut session, &mut starting, launch.page)?;
    quit_when_closed(&mut session);
    session.perf.running(session.state.perf);
    session.perf.view.output("Upload");
    let now = Instant::now();
    let sampled = session.state.perf && session.perf.wait(now).is_zero();
    if sampled {
      session.perf.sample(now);
    }
    if let Some(home) = &mut session.home {
      dirty |= home.updates();
    }
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
    let wait = wait_time(&mut session, &chores, animation_due, Instant::now());
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
    match drain(&mut session, inputs, &mut chores, &mut screen, pending) {
      Some(applied) => dirty |= applied,
      None => break,
    }
  }
  session.finish()
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

fn finish_launch(
  session: &mut Session,
  starting: &mut bool,
  page: Option<usize>,
) -> Result<()> {
  if !*starting || session.unlock.is_some() || !session.native_opens.is_empty()
  {
    return Ok(());
  }
  if !session.tabs.is_empty() {
    session.start_at(page)?;
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
  session.app().request = Some(super::Request::Keys(keys.into()));
  let result = session.settle();
  if result.is_err() || session.quit || session.tabs.is_empty() {
    session.pending_keys.clear();
  }
  report(session, result);
  true
}

fn picker_update(session: &mut Session, screen: Screen) -> bool {
  use std::task::Poll;
  let Some(picker) = &mut session.picker else { return false };
  let result = picker.poll();
  if matches!(result, Ok(Poll::Pending)) {
    return false;
  }
  session.picker = None;
  let result = result.and_then(|picked| {
    let Poll::Ready(Some(path)) = picked else { return Ok(()) };
    picked_path(session, path, screen)
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
  screen: Screen,
) -> Result<()> {
  let operation = if session.tabs.is_empty() {
    Operation::Open(path)
  } else {
    Operation::Request(super::Request::Open { path, tab: false })
  };
  session.resume(operation, screen)
}

fn animate(session: &mut Session, due: &mut Instant, now: Instant) -> bool {
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
  if !session.blocked()
    && (!session.native_opens.is_empty() || !session.pending_keys.is_empty())
  {
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
  }
  *output = Some(frame);
}

fn apply(session: &mut Session, input: Input, screen: &mut Screen) -> bool {
  if session.picker.is_some()
    && matches!(
      input,
      Input::Key(_)
        | Input::Paste(_)
        | Input::Mouse { .. }
        | Input::History { .. }
    )
  {
    return false;
  }
  match session.password_input(&input, *screen) {
    Ok(true) => {
      return !matches!(input, Input::Mouse { .. } | Input::History { .. });
    }
    Err(error) => {
      report(session, Err(error));
      return true;
    }
    Ok(false) => {}
  }
  let applied = match input {
    Input::Resize(size) => {
      *screen = size;
      session.resize(size).map(|()| true)
    }
    Input::Open(path) => {
      if session.blocked() {
        session.native_opens.push_back(path);
        Ok(true)
      } else {
        session.resume(Operation::Open(path), *screen).map(|()| true)
      }
    }
    Input::Key(key) if session.tabs.is_empty() => {
      home_event(session, vpdf_model::input::Event::Key(key), *screen)
        .map(|()| true)
    }
    Input::Paste(text) if session.tabs.is_empty() => {
      home_event(session, vpdf_model::input::Event::Paste(text), *screen)
        .map(|()| true)
    }
    Input::Key(key) => session.interact(Interaction::Key(key)),
    Input::Paste(text) => session.interact(Interaction::Paste(text)),
    Input::Mouse { event, pixels: (x, y), screen: drawn }
      if drawn == *screen =>
    {
      let mut report = vpdf_model::input::Mouse::from(event);
      report.at = vpdf_model::input::Spot::Pixel { x, y };
      if session.tabs.is_empty() {
        if matches!(
          event.kind,
          vpdf_model::input::MouseEventKind::Moved
            | vpdf_model::input::MouseEventKind::Drag(_)
        ) {
          Ok(false)
        } else {
          home_event(session, vpdf_model::input::Event::Mouse(report), *screen)
            .map(|()| true)
        }
      } else {
        session.interact(Interaction::Mouse(report))
      }
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

// Like the terminal, closing the last tab exits.
fn quit_when_closed(session: &mut Session) {
  if session.tabs.is_empty()
    && session.home.is_none()
    && session.unlock.is_none()
  {
    session.quit = true;
  }
}

fn ensure_home(session: &mut Session, screen: Screen) {
  if session.tabs.is_empty() && session.home.is_none() && !session.quit {
    session.home = Some(Home::new(
      &session.state.recent,
      session.content(screen),
      &session.frontend.wake(),
    ));
    let resized = session.resize(screen);
    report(session, resized);
  }
}

fn report(session: &mut Session, result: Result<()>) {
  if let Err(error) = result {
    if let Some(home) = &mut session.home {
      home.view.fail(error.to_string());
    } else if !session.tabs.is_empty() {
      session.app().error = Some(error.to_string());
    }
  }
}

fn home_event(
  session: &mut Session,
  event: vpdf_model::input::Event,
  screen: Screen,
) -> Result<()> {
  let Some(home) = &mut session.home else { return Ok(()) };
  home.configure(&session.settings);
  match home.native_event(event, &mut session.history)? {
    crate::home::Outcome::Open(path) => {
      session.resume(Operation::Open(path), screen)?;
    }
    crate::home::Outcome::Help(topic) => {
      session.request_help(
        &topic,
        vpdf_ui::topics::Invocation::Home,
        screen,
      )?;
    }
    crate::home::Outcome::Quit => session.quit = true,
    crate::home::Outcome::Pick => session.pick()?,
    crate::home::Outcome::Perf(on) => session.toggle_perf(on)?,
    crate::home::Outcome::ConfigReload => session.reload_config(),
    crate::home::Outcome::Theme(theme) => session.set_theme(theme),
    crate::home::Outcome::Stay => {}
  }
  Ok(())
}

fn completions(
  session: &mut Session,
  cache: &mut Cache<TileId, Arc<Vec<u8>>>,
  reloaded: &mut Vec<TileOwner>,
) -> bool {
  let mut dirty = session.image_updates(&mut std::io::sink());
  let mut evicted = Vec::new();
  for app in &mut session.tab().panes {
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
  if session.settings.theme() != vpdf_model::theme::Theme::Default {
    scene.push(Op::Theme(session.settings.theme()));
  }
  session.present_startup();
  if let Some(home) = &mut session.home {
    home.configure(&session.settings);
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
    if let Some(home) = &mut session.home {
      home.scene(&mut recorder)?;
    }
    "Vpdf".to_owned()
  } else {
    session.prepare_frame();
    let area = session.area();
    let tab = &mut session.tabs[session.active];
    Session::scene_tab(
      tab,
      area,
      session.unmeasured.as_ref(),
      &mut recorder,
      |app, scene| {
        app.scene_pane(scene, false)?;
        complete &= app.complete || app.covered();
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
  let scene = recorder.into_scene();
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
  let clipboard = session
    .panes_mut()
    .filter_map(|app| app.reader.take_yank())
    .filter(|yank| crate::clipboard::synced(yank.register))
    .fold(None, |_, yank| Some(yank.text));
  let cursor = if session.tabs.is_empty() || !vpdf_terminal::terminal::mouse() {
    vpdf_model::style::Shape::Default
  } else {
    session.shape()
  };
  Ok(Frame {
    metrics: frame_metrics(session, started, cache, &visible),
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
        Observation::of(&tab.panes[tab.focus])
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
  let apps = || tab.into_iter().flat_map(|tab| &tab.panes);
  let animating = apps().any(|app| app.reader.animation().active());
  let busy = apps().any(crate::app::App::busy);
  let focused = tab.map(|tab| &tab.panes[tab.focus]);
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

  fn wait(
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
      Bridge::configured(paths, store, page, config, move || wake.wake())?,
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
    Ok((root, path, session))
  }

  fn mouse(kind: MouseEventKind, pixels: (u16, u16), screen: Screen) -> Input {
    let (column, row) =
      (pixels.0 / screen.cell_width, pixels.1 / screen.cell_height);
    Input::Mouse { event: MouseEvent::new(kind, column, row), pixels, screen }
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
        session.app().reader.notice("Ordinary startup");
      }
      let warning = session.startup_warning.clone().unwrap();
      let rendered = frame(&mut session, &mut Cache::new(BUDGET), screen)?;
      assert!(shows(&rendered.scene, "Config"));
      if document {
        assert_eq!(session.app().reader.message, warning);
        assert!(!session.app().cursorline);
        assert_eq!(session.app().layout.zoom(), 2.0);
      } else {
        assert_eq!(
          session.home.as_ref().unwrap().view.error(),
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
      vec![encrypted.path().into()],
      None,
      Some(2),
      super::super::tests::settings_file(directory.path()),
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
    session.app().command("vsplit")?;
    session.settle()?;
    let tab = &mut session.tabs[session.active];
    tab.panes[1 - tab.focus].reader.animate_to(100.0, started);
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
    session.app().reader.animate_to(100.0, start);
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
      assert_eq!(session.app().reader.scroll, 0.0);
      assert_eq!(due, start + ANIMATION);
    }
    assert!(animate(&mut session, &mut due, start + ANIMATION));
    let scroll = session.app().reader.scroll;
    assert!(scroll > 0.0);
    assert!(!animate(&mut session, &mut due, start + ANIMATION));
    assert_eq!(session.app().reader.scroll, scroll);
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
    session.app().reader.cancel_scroll();
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
      assert_eq!(left.reader.cursor, right.reader.cursor);
      assert_eq!(left.reader.scroll, right.reader.scroll);
      assert_eq!(left.reader.mode(), right.reader.mode());
      assert_eq!(left.reader.selection, right.reader.selection);
      assert_eq!(left.layout.zoom(), right.layout.zoom());
      assert_eq!(left.error, right.error);
      assert_eq!(
        left.overlays.prompt().map(vpdf_ui::prompt::Prompt::text),
        right.overlays.prompt().map(vpdf_ui::prompt::Prompt::text)
      );
      assert!(left.request.is_none() && right.request.is_none());
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
      assert!(terminal.app().error.is_none(), "terminal step {step}");
      assert!(native.app().error.is_none(), "native step {step}");
      if let Some((_, page)) = pages.iter().find(|(at, _)| *at == step) {
        assert_eq!(terminal.tab().panes.len(), 2, "step {step}");
        assert_eq!(terminal.app().reader.cursor.page, *page, "step {step}");
      }
      if step == pasted || step == undone {
        let expected = if step == pasted { "章a\u{301}" } else { "" };
        assert_eq!(terminal.app().overlays.prompt().unwrap().text(), expected);
      }
      assert_sessions_match(&mut terminal, &mut native);
    }
    assert_eq!(terminal.tab().focus, 0);
    assert!(terminal.app().overlays.prompt().is_none());
    Ok(())
  }

  #[test]
  fn guarded_inputs_keep_requests() -> Result<()> {
    let mut screen = sized();
    let (_root, path, mut session) = opened(&["body"], screen)?;
    let stale = Screen { columns: screen.columns - 1, ..screen };
    session.app().request = Some(super::super::Request::Perf(Some(true)));
    for input in [
      Input::Leave,
      Input::Focus(false),
      Input::Resize(screen),
      Input::History { forward: false, pixels: (20, 20), screen: stale },
      mouse(MouseEventKind::Moved, (20, 20), stale),
    ] {
      apply(&mut session, input, &mut screen);
      assert!(!session.state.perf);
      assert!(session.app().request.is_some());
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
      assert!(session.app().request.is_some());
      assert_eq!(session.pending_keys.len(), 1);
      assert!(!resume_keys(&mut session));
    }
    assert_eq!(session.unlock.as_ref().unwrap().prompt.text(), "secretx");
    Ok(())
  }

  fn protected(path: &std::path::Path) -> Result<tempfile::NamedTempFile> {
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
      session.tabs[0].panes[0].quit = true;
      session.app().request = Some(Request::TabClose(None));
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
      assert!(session.app().quit);
      assert!(session.app().error.is_none());
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
      session.app().request = Some(Request::SetMark('a'));
      if native {
        apply(&mut session, Input::Leave, &mut screen);
      } else {
        assert!(run::apply(&mut session, Act::Leave, &mut Vec::new()));
      }
      assert!(session.pointer.target().is_none());
      assert_eq!(session.pointer.holding(), !native);
      assert_eq!(session.app().request.is_some(), native);
      assert_eq!(session.state.marks.contains_key(&'a'), !native);
      assert!(session.app().error.is_none());
    }
    Ok(())
  }

  /// A frame the window never took hands its one-shot fields on.
  #[test]
  fn publish_merges_untaken_frames() {
    let frame = |owner: u32, clipboard: Option<&str>| Frame {
      metrics: Metrics::default(),
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
        file: Some(path.clone()),
      },
      Request::GoMark('A', true),
      Request::PanelMark { name: 'A', focus: false },
    ] {
      let mut screen = sized();
      let mut session = native();
      session.open(&source, true, screen)?;
      let original = session.app().id;
      let position =
        Position { page: vpdf_model::text::PageId(1), ..Position::default() };
      session
        .state
        .marks
        .insert('A', Mark { path: path.clone(), cursor: position });
      session.app().request = Some(request.clone());
      session.settle()?;
      assert!(session.unlock.is_some(), "{request:?}");
      assert_eq!(session.app().id, original);
      apply(&mut session, Input::Paste("wrong".into()), &mut screen);
      apply(&mut session, key(KeyCode::Enter), &mut screen);
      assert!(session.unlock.as_ref().unwrap().retry);
      assert_eq!(session.app().id, original);
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
        assert_ne!(session.app().id, original);
      }
      Request::Window { .. } => {
        assert_eq!(session.tabs.len(), 1);
        assert_eq!(session.tab().panes.len(), 2);
        assert_eq!(session.app().path, path);
      }
      Request::PanelMark { .. } => {
        assert_eq!(session.app().id, original);
        assert_eq!(session.tab().panes.len(), 2);
      }
      _ => assert_eq!(session.tabs.len(), 2),
    }
    if matches!(request, Request::GoMark(..) | Request::PanelMark { .. }) {
      assert_eq!(
        session.tabs[target.0].panes[target.1].reader.cursor.page,
        position.page
      );
    }
  }

  #[test]
  fn password_prompt_lifecycle() -> Result<()> {
    let (_root, path) = temp_pdf(&["first"])?;
    let encrypted = protected(&path)?;
    let mut screen = sized();
    let mut session = native();
    ensure_home(&mut session, screen);
    apply(&mut session, Input::Open(encrypted.path().into()), &mut screen);
    assert!(session.home.is_some());
    apply(&mut session, Input::Paste("secret".into()), &mut screen);
    let mut cache = Cache::new(BUDGET);
    let drawn = frame(&mut session, &mut cache, screen)?;
    assert!(masked(&drawn));
    assert!(!shows(&drawn.scene, "secret"));
    apply(&mut session, Input::Focus(false), &mut screen);
    assert_eq!(session.unlock.as_ref().unwrap().prompt.text(), "secret");
    apply(&mut session, Input::Open(path.clone()), &mut screen);
    assert_eq!(session.native_opens.front(), Some(&path));
    assert!(session.tabs.is_empty());
    apply(&mut session, key(KeyCode::Esc), &mut screen);
    assert!(session.unlock.is_none());
    assert!(session.tabs.is_empty());
    assert!(session.home.is_some() && !session.quit);
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
      session.app().request = Some(Request::Keys(keys));
      session.settle()?;
      assert!(session.unlock.is_some());
      assert_eq!(session.pending_keys.len(), 3);
      if cancel {
        apply(&mut session, key(KeyCode::Esc), &mut screen);
        assert!(!resume_keys(&mut session));
        assert_eq!(session.app().path, std::fs::canonicalize(&source)?);
      } else {
        apply(&mut session, Input::Paste("test-reader".into()), &mut screen);
        apply(&mut session, key(KeyCode::Enter), &mut screen);
        assert!(resume_keys(&mut session));
        assert_eq!(
          session.app().reader.cursor.page,
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
    session.app().request = Some(super::super::Request::Keys(
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
  fn home_reserves_perf_and_last_close_quits() -> Result<()> {
    let (_root, path) = temp_pdf(&["first"])?;
    let mut screen = sized();
    let mut session = native();
    session.state.set_perf(true);
    ensure_home(&mut session, screen);
    apply(&mut session, Input::Focus(true), &mut screen);
    assert_eq!(
      session.home.as_ref().unwrap().view.screen(),
      session.content(screen)
    );
    assert!(session.home.as_ref().unwrap().view.screen().origin.1 > 0);
    apply(
      &mut session,
      Input::Open(PathBuf::from("/missing.pdf")),
      &mut screen,
    );
    assert!(session.home.is_some() && !session.quit);
    apply(&mut session, Input::Open(path), &mut screen);
    assert!(session.home.is_none());
    apply(&mut session, key(KeyCode::Char('q')), &mut screen);
    assert!(session.tabs.is_empty() && session.quit);
    Ok(())
  }

  #[test]
  fn help_refresh_invalidates_native_tiles() -> Result<()> {
    let screen = sized();
    let mut session = native();
    session.request_help(
      "config",
      vpdf_ui::topics::Invocation::Home,
      screen,
    )?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while session.tabs.is_empty() {
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
    for &owner in &owners {
      cache.insert(
        TileId { owner, key: vpdf_model::tile::TileKey::default() },
        Arc::new(vec![255, 0, 0]),
        3,
      );
    }
    session.set_theme(vpdf_model::theme::Theme::Nord);
    while session.app().document_generation == 0 {
      session.help_updates(&mut std::io::sink());
      assert!(Instant::now() < deadline, "Help refresh timed out");
      thread::sleep(Duration::from_millis(2));
    }
    let mut reloaded = Vec::new();
    assert!(completions(&mut session, &mut cache, &mut reloaded));
    for owner in owners {
      assert!(reloaded.contains(&owner));
      assert!(
        cache
          .get(&TileId { owner, key: vpdf_model::tile::TileKey::default() })
          .is_none()
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
    let mut screen = sized();
    let mut session = native();
    picked_path(&mut session, first.clone(), screen)?;
    assert_eq!(session.tabs.len(), 1);
    picked_path(&mut session, second.clone(), screen)?;
    assert_eq!(session.tabs.len(), 1);
    assert_eq!(session.app().path, std::fs::canonicalize(&second)?);
    apply(&mut session, Input::Open(first), &mut screen);
    assert_eq!(session.tabs.len(), 2);
    picked_path(&mut session, second, screen)?;
    assert_eq!(session.tabs.len(), 2);
    assert_eq!(session.active, 0);
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
      [(None, 2), (Some(0), 0), (Some(1), 0), (Some(2), 1), (Some(99), 2)]
    {
      let mut saved = Session::new(Some(store.clone()));
      saved.open(&first, true, sized())?;
      saved.start_at(Some(3))?;
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
    session.app().command("tabnew")?;
    session.settle()?;
    for command in ["vsplit", "split"] {
      session.app().command(command)?;
      session.settle()?;
    }
    session.focus(session.active, focus)?;
    for app in &mut session.tabs[session.active].panes {
      app.close_panel()?;
      match view {
        "page" => {}
        "covered" => {
          app.show_recent(&[])?;
          assert!(app.covered());
        }
        "overview" => app.open_overview()?,
        _ => app.text_mode = true,
      }
    }
    if view == "prompt" {
      session.app().action(Input::plain(Action::Command))?;
      session.app().overlays.prompt_mut().unwrap().paste("set ");
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
        let retained = frame(&mut recorded, &mut cache, screen)?;
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
          assert_eq!(left.complete, right.complete, "{view}");
          assert_eq!(left.tiles.wanted(), right.tiles.wanted(), "{view}");
          assert_eq!(left.outline_pending(), right.outline_pending(), "{view}");
        }
        let tab = &recorded.tabs[recorded.active];
        assert_eq!(
          retained.complete,
          screen.rows != 0
            && tab.panes.iter().all(|app| app.complete || app.covered())
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
        tab.panes[index].draw_pane(&mut session.graphics, &mut out)?;
      }
    }
    crate::presentation::draw(&mut out, None, |scene| {
      tab.tree.scene_dividers(scene, area);
    })?;
    tab.panes[tab.focus].draw_pane(&mut session.graphics, &mut out)?;
    if let Some(notice) = &session.unmeasured {
      crate::presentation::draw(&mut out, None, |scene| notice.scene(scene))?;
    }
    session.graphics.end(&mut out)?;
    write!(out, "{}", terminal::frame_end())?;
    session.finish_frame();
    Ok(out)
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
      for app in &mut session.tabs[session.active].panes {
        for key in vpdf_ui::layout::visible(&app.layout, app.reader.scroll) {
          let tile = crate::raster::render(&mut app.doc, key)?;
          session.graphics.keep(app.id, tile);
        }
      }
    }
    for app in &mut recorded.tabs[recorded.active].panes {
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
    let focused_bytes = resident.bytes - streamed.app().image_usage.bytes;
    for session in [&mut streamed, &mut recorded, &mut legacy] {
      session.drawn();
      assert!(
        session.tabs[session.active]
          .panes
          .iter()
          .all(|app| !matches!(app.outline_load, Loading::Deferred))
      );
    }
    assert!(streamed.app().image_usage.uploaded > 0);
    streamed.app().images_stale = true;
    legacy.app().images_stale = true;
    assert_frame_usage(&mut streamed, &mut legacy)?;
    let tab = &streamed.tabs[streamed.active];
    assert_eq!(tab.panes[1].image_usage.bytes, resident.bytes);
    assert_eq!(tab.panes[2].image_usage.bytes, resident.bytes);
    assert_eq!(tab.panes[0].image_usage.bytes, resident.bytes - focused_bytes);
    assert!(!tab.panes[0].images_stale);
    assert_eq!(streamed.graphics.usage().bytes, resident.bytes - focused_bytes);
    assert_eq!(streamed.graphics.usage().uploaded, resident.uploaded);
    Ok(())
  }

  fn assert_waiting(session: &Session) {
    for app in &session.tabs[session.active].panes {
      assert!(!app.complete);
      assert!(matches!(app.outline_load, crate::jobs::Loading::Deferred));
    }
  }

  fn assert_upload_order(session: &Session) {
    let tab = &session.tabs[session.active];
    let samples = [
      tab.panes[1].image_usage,
      tab.panes[2].image_usage,
      tab.panes[0].image_usage,
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
      assert_eq!(left.image_usage.bytes, right.image_usage.bytes);
      assert_eq!(left.image_usage.uploaded, right.image_usage.uploaded);
      assert_eq!(left.complete, right.complete);
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
    session.app().show_preview(Target::Page(PageId(1)), Position::default())?;
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
    session.app().overlays.close_preview();
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
    let app = session.app();
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
    let old_page = session.app().reader.cursor.page;
    let stale = Screen { columns: 119, ..screen };
    apply(
      &mut session,
      mouse(MouseEventKind::Down(MouseButton::Left), pixels, stale),
      &mut screen,
    );
    assert!(!session.pointer.holding());
    assert_eq!(session.app().reader.cursor.page, old_page);
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
    assert_eq!(session.app().reader.cursor.page, target);
    apply(
      &mut session,
      Input::History { forward: false, pixels, screen: drawn },
      &mut screen,
    );
    assert_eq!(session.app().reader.cursor.page, source.page);
    apply(
      &mut session,
      Input::History { forward: true, pixels, screen: drawn },
      &mut screen,
    );
    assert_eq!(session.app().reader.cursor.page, target);
    apply(&mut session, Input::Focus(false), &mut screen);
    assert!(!session.pointer.holding());
    assert!(session.pointer.target().is_none());
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
    session.home =
      Some(Home::new(&session.state.recent, screen, &Wake::none()));
    session.resize(screen)?;
    frame(&mut session, &mut Cache::new(BUDGET), screen)?;
    for kind in [
      MouseEventKind::Down(MouseButton::Left),
      MouseEventKind::Up(MouseButton::Left),
    ] {
      apply(&mut session, mouse(kind, (32, 20), screen), &mut screen);
    }
    assert_eq!(session.tabs.len(), 1);
    assert_eq!(session.app().path, std::fs::canonicalize(path)?);
    assert!(session.home.is_none());
    Ok(())
  }

  #[test]
  fn wheel_routes_to_panels_and_pointer_zoom() -> Result<()> {
    use vpdf_ui::keys::PanelKind;
    let mut screen = cells(80, 24);
    let (_root, _, mut session) = opened(&["first", "second"], screen)?;
    session.app().show_panel(
      PanelKind::Version,
      (0..80)
        .map(|row| vpdf_ui::panel::Row::parse(&format!("row {row}")))
        .collect(),
    )?;
    let mut cache = Cache::new(BUDGET);
    frame(&mut session, &mut cache, screen)?;
    let scroll = session.app().reader.scroll;
    let down = mouse(MouseEventKind::ScrollDown, (8, 80), screen);
    apply(&mut session, down, &mut screen);
    assert!(session.app().panel.as_ref().unwrap().view.top() > 0);
    assert_eq!(session.app().reader.scroll, scroll);
    session.app().close_panel()?;
    let zoom = session.app().layout.zoom();
    let moved = mouse(MouseEventKind::Moved, (320, 100), screen);
    apply(&mut session, moved, &mut screen);
    let mut up = mouse(MouseEventKind::ScrollUp, (320, 100), screen);
    if let Input::Mouse { event, .. } = &mut up {
      event.modifiers = KeyModifiers::CONTROL;
    }
    apply(&mut session, up, &mut screen);
    assert!(session.app().layout.zoom() > zoom);
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
        .reader
        .deliver_yank(Yank { text: "first".into(), register: '"' });
      session
        .app()
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
    session.app().select_region(
      PageId(0),
      Bounds { x0: 10.0, y0: 10.0, x1: 50.0, y1: 50.0 },
    )?;
    session.app().region_operation(Operation::Save)?;
    session
      .app()
      .reader
      .deliver_yank(Yank { text: "pending region text".into(), register: '"' });
    session.open(&second, true, screen)?;
    let mut cache = Cache::new(BUDGET);
    let deadline = Instant::now() + Duration::from_secs(10);
    while session.tabs[0].panes[0].region_job.is_some() {
      wakes.recv_timeout(deadline.saturating_duration_since(Instant::now()))?;
      completions(&mut session, &mut cache, &mut Vec::new());
    }
    assert!(session.tabs[0].panes[0].reader.message.starts_with("Saved "));
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
    assert!(session.home.as_ref().unwrap().view.typing());
    apply(&mut session, Input::Open(first.clone()), &mut screen);
    assert!(session.home.is_none());
    session.app().command("vsplit")?;
    session.settle()?;
    let owners: Vec<_> = session.tab().panes.iter().map(|app| app.id).collect();
    apply(&mut session, key(KeyCode::Char(':')), &mut screen);
    assert!(session.app().overlays.prompt().is_some());
    apply(&mut session, Input::Open(second.clone()), &mut screen);
    assert_eq!(session.tabs.len(), 2);
    assert_eq!(session.active, 1);
    assert_eq!(session.app().path, std::fs::canonicalize(second)?);
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
    assert_eq!(session.app().overlays.prompt().unwrap().text(), "章a\u{301}");
    apply(
      &mut session,
      Input::Key(KeyEvent::new(KeyCode::Char('_'), KeyModifiers::CONTROL)),
      &mut screen,
    );
    assert_eq!(session.app().overlays.prompt().unwrap().text(), "");
    Ok(())
  }

  #[test]
  fn overflow_restarts_delivery_and_keeps_received_pixels() -> Result<()> {
    use vpdf_pdf::jobs::render::Rendered;
    let screen = crate::screen::pixels(60, 20, 10, 20);
    let (_root, _, mut session) = opened(&["body"], screen)?;
    let app = session.app();
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
}
