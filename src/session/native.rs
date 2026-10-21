//! Session ownership for native windows.
use super::interaction::Interaction;
use super::maintenance::Chores;
use super::unlock::Operation;
use super::{Home, Screen, Session};
use crate::Result;
#[cfg(test)]
use crate::ui_presentation::*;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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
  #[cfg(feature = "bench-events")]
  Mark {
    name: String,
    trace: Arc<crate::trace::Log>,
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

pub struct Frame {
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
  send: mpsc::Sender<Input>,
  latest: Arc<Mutex<Option<Frame>>>,
  thread: Option<JoinHandle<Result<()>>>,
}

struct Launch {
  paths: Vec<PathBuf>,
  store: Option<PathBuf>,
  page: Option<usize>,
}

impl Bridge {
  pub fn start(
    paths: Vec<PathBuf>,
    page: Option<usize>,
    repaint: impl Fn() + Send + Sync + 'static,
  ) -> Result<Self> {
    Self::stored(paths, crate::state::location(), page, repaint)
  }

  fn stored(
    paths: Vec<PathBuf>,
    store: Option<PathBuf>,
    page: Option<usize>,
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
        let _ = wake_send.send(Input::Wake);
      }
    });
    let worker =
      thread::Builder::new().name("vpdf-session".into()).spawn(move || {
        run(
          Launch { paths, store, page },
          &receive,
          &output,
          &repaint,
          &wake,
          &pending,
        )
      })?;
    Ok(Self { send, latest, thread: Some(worker) })
  }

  pub fn send(&self, input: Input) {
    let _ = self.send.send(input);
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
  receive: &mpsc::Receiver<Input>,
  latest: &Mutex<Option<Frame>>,
  repaint: &impl Fn(),
  wake: &Wake,
  pending: &AtomicBool,
) -> Result<()> {
  let mut starting = !launch.paths.is_empty();
  let mut session = Session::new(launch.store);
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
  loop {
    dirty |= picker_update(&mut session, screen);
    dirty |= resume_keys(&mut session);
    if session.unlock.is_none()
      && session.picker.is_none()
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
    let inputs = std::iter::once(input).chain(receive.try_iter().take(BATCH));
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
      Input::Mark { name, trace } => {
        if let Err(error) = trace.mark(Instant::now(), &name) {
          eprintln!("Trace mark failed: {error}");
        }
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
        apply(session, input, screen);
        applied = true;
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
  if session.unlock.is_some() || session.picker.is_some() {
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

const ANIMATION: Duration = Duration::from_millis(8);

fn animate(session: &mut Session, due: &mut Instant, now: Instant) -> bool {
  if !session.animating() {
    *due = now;
    return false;
  }
  if now < *due {
    return false;
  }
  *due = now + ANIMATION;
  session.animate(now)
}

fn wait_time(
  session: &mut Session,
  chores: &Chores,
  animation_due: Instant,
  now: Instant,
) -> Duration {
  if session.unlock.is_none()
    && session.picker.is_none()
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
  wait
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

fn apply(session: &mut Session, input: Input, screen: &mut Screen) {
  if session.picker.is_some()
    && matches!(
      input,
      Input::Key(_)
        | Input::Paste(_)
        | Input::Mouse { .. }
        | Input::History { .. }
    )
  {
    return;
  }
  match session.password_input(&input, *screen) {
    Ok(true) => return,
    Err(error) => {
      report(session, Err(error));
      return;
    }
    Ok(false) => {}
  }
  let applied = match input {
    Input::Resize(size) => {
      *screen = size;
      session.resize(size)
    }
    Input::Open(path) => {
      if session.unlock.is_some() || session.picker.is_some() {
        session.native_opens.push_back(path);
        Ok(())
      } else {
        session.resume(Operation::Open(path), *screen)
      }
    }
    Input::Key(key) if session.tabs.is_empty() => {
      home_event(session, vpdf_model::input::Event::Key(key), *screen)
    }
    Input::Paste(text) if session.tabs.is_empty() => {
      home_event(session, vpdf_model::input::Event::Paste(text), *screen)
    }
    Input::Key(key) => session.interact(Interaction::Key(key)).map(drop),
    Input::Paste(text) => session.interact(Interaction::Paste(text)).map(drop),
    Input::Mouse { event, pixels: (x, y), screen: drawn }
      if drawn == *screen =>
    {
      let mut report = vpdf_model::input::Mouse::from(event);
      report.at = vpdf_model::input::Spot::Pixel { x, y };
      if session.tabs.is_empty() {
        home_event(session, vpdf_model::input::Event::Mouse(report), *screen)
      } else {
        session.interact(Interaction::Mouse(report)).map(drop)
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
      session.interact(Interaction::Mouse(report)).map(drop)
    }
    Input::Leave => {
      session.interact(Interaction::Leave).map(|_| session.drop_pointer())
    }
    Input::Focus(focused) => {
      session.sight =
        if focused { super::Sight::Focused } else { super::Sight::Unfocused };
      if !focused {
        session.pointer_left();
        session.drop_pointer();
      }
      Ok(())
    }
    _ => Ok(()),
  };
  quit_when_closed(session);
  report(session, applied);
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
    session.home = Some(Home::with_wake(
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
  match home.native_event(event, &mut session.history)? {
    crate::home::Outcome::Open(path) => {
      session.resume(Operation::Open(path), screen)?;
    }
    crate::home::Outcome::Quit => session.quit = true,
    crate::home::Outcome::Pick => session.pick()?,
    crate::home::Outcome::Perf(on) => session.toggle_perf(on)?,
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
  for app in session.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
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

fn frame(
  session: &mut Session,
  cache: &mut Cache<TileId, Arc<Vec<u8>>>,
  screen: Screen,
) -> Result<Frame> {
  let owners: Vec<_> = session
    .tabs
    .iter()
    .flat_map(|tab| &tab.panes)
    .map(|app| TileOwner::new(app.id))
    .collect();
  let retired: Vec<_> = cache
    .entries()
    .filter(|(id, _)| !owners.contains(&id.owner))
    .map(|(id, _)| *id)
    .collect();
  for id in retired {
    cache.remove(&id);
  }
  let mut recorder = Recorder::new(|id| cache.get(&id).is_some());
  let mut complete = true;
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
    .tabs
    .iter_mut()
    .flat_map(|tab| &mut tab.panes)
    .filter_map(|app| app.reader.take_yank())
    .filter(|yank| crate::clipboard::synced(yank.register))
    .fold(None, |_, yank| Some(yank.text));
  let cursor = if session.tabs.is_empty() || !vpdf_terminal::terminal::mouse() {
    vpdf_model::style::Shape::Default
  } else {
    session.shape()
  };
  Ok(Frame {
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

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_model::input::{KeyCode, KeyModifiers};

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

  fn sized() -> Screen {
    Screen {
      columns: 100,
      rows: 40,
      cell_width: 10,
      cell_height: 20,
      origin: (0, 0),
    }
  }

  #[test]
  fn unrelated_wakes_keep_animation_deadline() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("animation.pdf");
    crate::fixtures::pdf(&path, &["first", "second"], "")?;
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
    session.open(&path, true, sized())?;
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
      assert!(!animate(
        &mut session,
        &mut due,
        start + Duration::from_millis(step)
      ));
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
    use vpdf_model::input::MouseEventKind;
    use vpdf_model::input::{Button, Mouse, MouseKind, Spot};
    use vpdf_model::text::PageId;
    let path = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
    let mut screen = sized();
    let mut terminal = Session::new(None);
    let mut native = Session::new(None);
    native.frontend = crate::raster::Frontend::Native(Wake::none());
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
    use vpdf_model::input::MouseEventKind;
    let root = tempfile::tempdir()?;
    let path = root.path().join("guard.pdf");
    crate::fixtures::pdf(&path, &["body"], "")?;
    let mut screen = sized();
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
    session.open(&path, true, screen)?;
    let stale = Screen { columns: screen.columns - 1, ..screen };
    session.app().request = Some(super::super::Request::Perf(Some(true)));
    for input in [
      Input::Leave,
      Input::Focus(false),
      Input::Resize(screen),
      Input::History { forward: false, pixels: (20, 20), screen: stale },
      Input::Mouse {
        event: MouseEvent {
          kind: MouseEventKind::Moved,
          column: 2,
          row: 1,
          modifiers: KeyModifiers::NONE,
        },
        pixels: (20, 20),
        screen: stale,
      },
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
      let mut session = Session::new(None);
      let mut screen = sized();
      if native {
        session.frontend = crate::raster::Frontend::Native(Wake::none());
      }
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
          Input::Mouse {
            event: MouseEvent {
              kind: vpdf_model::input::MouseEventKind::Moved,
              column: 0,
              row: 0,
              modifiers: KeyModifiers::NONE,
            },
            pixels: (0, 0),
            screen,
          },
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
    use vpdf_model::input::{MouseButton, MouseEventKind};
    let root = tempfile::tempdir()?;
    let path = root.path().join("leave.pdf");
    crate::fixtures::pdf(&path, &["body"], "")?;
    for native in [false, true] {
      let mut session = Session::new(None);
      let mut screen = sized();
      if native {
        session.frontend = crate::raster::Frontend::Native(Wake::none());
      }
      session.open(&path, true, screen)?;
      for kind in
        [MouseEventKind::Moved, MouseEventKind::Down(MouseButton::Left)]
      {
        session.mouse(MouseEvent {
          kind,
          column: 10,
          row: 8,
          modifiers: KeyModifiers::NONE,
        })?;
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
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.pdf");
    crate::fixtures::pdf(&source, &["first", "second"], "")?;
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
      let mut session = Session::new(None);
      session.frontend = crate::raster::Frontend::Native(Wake::none());
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
    let root = tempfile::tempdir()?;
    let path = root.path().join("source.pdf");
    crate::fixtures::pdf(&path, &["first"], "")?;
    let encrypted = protected(&path)?;
    let mut screen = sized();
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
    ensure_home(&mut session, screen);
    apply(&mut session, Input::Open(encrypted.path().into()), &mut screen);
    assert!(session.home.is_some());
    apply(&mut session, Input::Paste("secret".into()), &mut screen);
    let mut cache = Cache::new(BUDGET);
    let drawn = frame(&mut session, &mut cache, screen)?;
    assert!(
      drawn
        .scene
        .iter()
        .any(|op| matches!(op, Op::InputArea { masked: true, .. }))
    );
    assert!(!drawn.scene.iter().any(|op| matches!(op,
      Op::Text(text) if text.as_str().contains("secret"))));
    apply(&mut session, Input::Focus(false), &mut screen);
    assert_eq!(session.unlock.as_ref().unwrap().prompt.text(), "secret");
    apply(&mut session, Input::Open(path.clone()), &mut screen);
    assert_eq!(session.native_opens.front(), Some(&path));
    assert!(session.tabs.is_empty());
    apply(&mut session, key(KeyCode::Esc), &mut screen);
    assert!(session.unlock.is_none());
    assert!(session.tabs.is_empty());
    assert!(session.home.is_some() && !session.quit);
    let (send, wakes) = mpsc::channel();
    let mut bridge =
      Bridge::stored(vec![encrypted.path().into()], None, None, move || {
        let _ = send.send(());
      })?;
    wait(&bridge, &wakes, |frame| {
      frame
        .scene
        .iter()
        .any(|op| matches!(op, Op::InputArea { masked: true, .. }))
    })?;
    bridge.close()?;
    Ok(())
  }

  #[test]
  fn protected_keys_resume() -> Result<()> {
    use super::super::Request;
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.pdf");
    crate::fixtures::pdf(&source, &["first", "second"], "")?;
    let encrypted = protected(&source)?;
    for cancel in [false, true] {
      let mut screen = sized();
      let mut session = Session::new(None);
      session.frontend = crate::raster::Frontend::Native(Wake::none());
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
    let root = tempfile::tempdir()?;
    let path = root.path().join("source.pdf");
    crate::fixtures::pdf(&path, &["first"], "")?;
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
    session.open(&path, true, sized())?;
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
    let root = tempfile::tempdir()?;
    let path = root.path().join("source.pdf");
    crate::fixtures::pdf(&path, &["first"], "")?;
    let mut screen = sized();
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
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
  fn native_help_offers_picker() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("source.pdf");
    crate::fixtures::pdf(&path, &["first"], "")?;
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
    session.open(&path, true, sized())?;
    let rows = session.app().panel_rows(super::super::PanelKind::Help)?;
    assert!(rows.iter().any(|row| row.label.contains("fzf")));
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
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
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
    let (send, wakes) = mpsc::channel();
    let mut bridge =
      Bridge::stored(Vec::new(), Some(store), None, move || {
        let _ = send.send(());
      })?;
    wait(&bridge, &wakes, |frame| frame.content)?;
    let sampled = wait(&bridge, &wakes, |frame| {
      !frame.content
        && frame.scene.iter().any(|op| {
          matches!(op,
        Op::Text(text) if text.as_str().contains("RSS ")
          && !text.as_str().contains("RSS —"))
        })
    })?;
    assert!(sampled.scene.iter().any(|op| matches!(op,
      Op::Text(text) if text.as_str().contains("FPS 0"))));
    bridge.send(Input::Presented { content: true, bytes: 2048 });
    wait(&bridge, &wakes, |frame| {
      frame.scene.iter().any(|op| {
        matches!(op,
      Op::Text(text) if text.as_str().contains("Upload 2.0K/s"))
      })
    })?;
    bridge.send(key(KeyCode::Char(':')));
    bridge.send(Input::Paste("perf off".into()));
    bridge.send(key(KeyCode::Enter));
    wait(&bridge, &wakes, |frame| {
      !frame.scene.iter().any(|op| {
        matches!(op,
      Op::Text(text) if text.as_str().contains("Upload "))
      })
    })?;
    bridge.close()
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn encrypted_corpus_renders() -> Result<()> {
    let path = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
    let encrypted = protected(&path)?;
    let (send, wakes) = mpsc::channel();
    let mut bridge =
      Bridge::stored(vec![encrypted.path().into()], None, None, move || {
        let _ = send.send(());
      })?;
    wait(&bridge, &wakes, |frame| {
      frame
        .scene
        .iter()
        .any(|op| matches!(op, Op::InputArea { masked: true, .. }))
    })?;
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
      let (send, wakes) = mpsc::channel();
      let mut bridge = Bridge::stored(
        vec![first.clone(), second.clone()],
        Some(store.clone()),
        page,
        move || {
          let _ = send.send(());
        },
      )?;
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
    let (send, wakes) = mpsc::channel();
    let mut bridge = Bridge::stored(
      vec![encrypted.path().into(), second],
      Some(store.clone()),
      Some(3),
      move || {
        let _ = send.send(());
      },
    )?;
    wait(&bridge, &wakes, |frame| {
      frame
        .scene
        .iter()
        .any(|op| matches!(op, Op::InputArea { masked: true, .. }))
    })?;
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
    let (send, wakes) = mpsc::channel();
    let mut bridge = Bridge::stored(
      vec![path.clone()],
      Some(store.clone()),
      None,
      move || {
        let _ = send.send(());
      },
    )?;
    let first = wait(&bridge, &wakes, |frame| {
      frame.complete && !frame.images.is_empty()
    })?;
    assert!(first.images.iter().all(|image| image.pixels.len()
      == image.id.key.width as usize * image.id.key.height as usize * 3));
    let screen = Screen {
      columns: 60,
      rows: 20,
      cell_width: 12,
      cell_height: 24,
      origin: (0, 0),
    };
    bridge.send(Input::Resize(screen));
    bridge
      .send(Input::Key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE)));
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
    let (send, wakes) = mpsc::channel();
    let mut home = Bridge::stored(Vec::new(), Some(store), None, move || {
      let _ = send.send(());
    })?;
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
    let screen = Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    for view in ["page", "text", "panel", "overview", "preview", "clues"] {
      let setup = || -> Result<App> {
        let mut app = App::with_screen(&path, "", view == "text", screen)?;
        match view {
          "panel" => {
            app.action(Input::plain(Action::Panel(PanelKind::Help)))?;
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
    tab.tree.draw_dividers(&mut out, area)?;
    tab.panes[tab.focus].draw_pane(&mut session.graphics, &mut out)?;
    if let Some(notice) = &session.unmeasured {
      notice.draw(&mut out)?;
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
    let screen = Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
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
    use vpdf_model::input::{MouseButton, MouseEventKind};
    use vpdf_model::text::PageId;
    let path = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
    let mut screen = Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
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
    let mouse = |kind, drawn: Screen| Input::Mouse {
      event: MouseEvent {
        kind,
        column: pixels.0 / drawn.cell_width,
        row: pixels.1 / drawn.cell_height,
        modifiers: KeyModifiers::NONE,
      },
      pixels,
      screen: drawn,
    };
    let mut cache = Cache::new(BUDGET);
    frame(&mut session, &mut cache, screen)?;
    let drawn = screen;
    apply(&mut session, mouse(MouseEventKind::Moved, drawn), &mut screen);
    assert_eq!(session.shape(), vpdf_model::style::Shape::Pointer);
    let old_page = session.app().reader.cursor.page;
    let stale = Screen { columns: 119, ..screen };
    apply(
      &mut session,
      mouse(MouseEventKind::Down(MouseButton::Left), stale),
      &mut screen,
    );
    assert!(!session.pointer.holding());
    assert_eq!(session.app().reader.cursor.page, old_page);
    for kind in [
      MouseEventKind::Down(MouseButton::Left),
      MouseEventKind::Up(MouseButton::Left),
    ] {
      apply(&mut session, mouse(kind, drawn), &mut screen);
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
    use vpdf_model::input::{MouseButton, MouseEventKind};
    let root = tempfile::tempdir()?;
    let path = root.path().join("recent.pdf");
    crate::fixtures::pdf(&path, &["recent"], "")?;
    let mut screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
    session.state.recent =
      vec![crate::state::tests::entry(&path.to_string_lossy(), 0, 0)];
    session.home =
      Some(Home::with_wake(&session.state.recent, screen, &Wake::none()));
    session.resize(screen)?;
    frame(&mut session, &mut Cache::new(BUDGET), screen)?;
    for kind in [
      MouseEventKind::Down(MouseButton::Left),
      MouseEventKind::Up(MouseButton::Left),
    ] {
      let input = Input::Mouse {
        event: MouseEvent {
          kind,
          column: 4,
          row: 1,
          modifiers: KeyModifiers::NONE,
        },
        pixels: (32, 20),
        screen,
      };
      apply(&mut session, input, &mut screen);
    }
    assert_eq!(session.tabs.len(), 1);
    assert_eq!(session.app().path, std::fs::canonicalize(path)?);
    assert!(session.home.is_none());
    Ok(())
  }

  #[test]
  fn wheel_routes_to_panels_and_pointer_zoom() -> Result<()> {
    use vpdf_model::input::MouseEventKind;
    use vpdf_ui::keys::PanelKind;
    let root = tempfile::tempdir()?;
    let path = root.path().join("wheel.pdf");
    crate::fixtures::pdf(&path, &["first", "second"], "")?;
    let mut screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
    session.open(&path, true, screen)?;
    session.app().open_panel(PanelKind::Help)?;
    let mut cache = Cache::new(BUDGET);
    frame(&mut session, &mut cache, screen)?;
    let scroll = session.app().reader.scroll;
    let mut input =
      |session: &mut Session, kind, pixels: (u16, u16), modifiers| {
        let event = Input::Mouse {
          event: MouseEvent {
            kind,
            column: pixels.0 / screen.cell_width,
            row: pixels.1 / screen.cell_height,
            modifiers,
          },
          pixels,
          screen,
        };
        apply(session, event, &mut screen);
      };
    input(
      &mut session,
      MouseEventKind::ScrollDown,
      (8, 80),
      KeyModifiers::NONE,
    );
    assert!(session.app().panel.as_ref().unwrap().view.top() > 0);
    assert_eq!(session.app().reader.scroll, scroll);
    session.app().close_panel()?;
    let zoom = session.app().layout.zoom();
    input(&mut session, MouseEventKind::Moved, (320, 100), KeyModifiers::NONE);
    input(
      &mut session,
      MouseEventKind::ScrollUp,
      (320, 100),
      KeyModifiers::CONTROL,
    );
    assert!(session.app().layout.zoom() > zoom);
    Ok(())
  }

  #[test]
  fn focus_loss_releases_active_pointer_capture() -> Result<()> {
    use vpdf_model::input::{MouseButton, MouseEventKind};
    let root = tempfile::tempdir()?;
    let path = root.path().join("pointer.pdf");
    crate::fixtures::pdf(&path, &["first", "second"], "")?;
    let mut screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
    session.open(&path, true, screen)?;
    let input = Input::Mouse {
      event: MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 10,
        row: 8,
        modifiers: KeyModifiers::NONE,
      },
      pixels: (80, 128),
      screen,
    };
    apply(&mut session, input, &mut screen);
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
    let screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
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
    let screen = Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
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
    let mut screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
    ensure_home(&mut session, screen);
    apply(&mut session, Input::Resize(screen), &mut screen);
    let colon =
      || Input::Key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE));
    apply(&mut session, colon(), &mut screen);
    assert!(session.home.as_ref().unwrap().view.typing());
    apply(&mut session, Input::Open(first.clone()), &mut screen);
    assert!(session.home.is_none());
    session.app().command("vsplit")?;
    session.settle()?;
    let owners: Vec<_> = session.tab().panes.iter().map(|app| app.id).collect();
    apply(&mut session, colon(), &mut screen);
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
    let root = tempfile::tempdir()?;
    let path = root.path().join("input.pdf");
    crate::fixtures::pdf(&path, &["body"], "")?;
    let mut screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::none());
    session.open(&path, true, screen)?;
    apply(
      &mut session,
      Input::Key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE)),
      &mut screen,
    );
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
    let root = tempfile::tempdir()?;
    let path = root.path().join("overflow.pdf");
    crate::fixtures::pdf(&path, &["body"], "")?;
    let mut session = Session::new(None);
    session.frontend = crate::raster::Frontend::Native(Wake::new(|| {}));
    let screen = Screen {
      columns: 60,
      rows: 20,
      cell_width: 10,
      cell_height: 20,
      origin: (0, 0),
    };
    session.open(&path, true, screen)?;
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
    let (send, wakes) = mpsc::channel();
    let mut bridge = Bridge::stored(Vec::new(), None, None, move || {
      let _ = send.send(());
    })?;
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
