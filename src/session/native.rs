//! Session ownership for native windows.
use super::{Home, Screen, Session};
use crate::Result;
use crossterm::event::{KeyEvent, MouseEvent};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use vpdf_model::cache::Cache;
use vpdf_model::scene::{Op, Recorder, Scene};
use vpdf_model::tile::{TileId, TileOwner};
use vpdf_pdf::jobs::Wake;

const BUDGET: usize = 64 << 20;

pub enum Input {
  Resize(Screen),
  Key(KeyEvent),
  Paste(String),
  Mouse { event: MouseEvent, pixels: (u16, u16), screen: Screen },
  History { forward: bool, pixels: (u16, u16), screen: Screen },
  Leave,
  Focus(bool),
  Open(PathBuf),
  Presented,
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
  pub title: String,
  pub cursor: vpdf_model::style::Shape,
  pub complete: bool,
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

impl Bridge {
  pub fn start(
    paths: Vec<PathBuf>,
    repaint: impl Fn() + Send + Sync + 'static,
  ) -> Result<Self> {
    Self::stored(paths, crate::state::location(), repaint)
  }

  fn stored(
    paths: Vec<PathBuf>,
    store: Option<PathBuf>,
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
        run(paths, store, &receive, &output, &repaint, &wake, &pending)
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
  paths: Vec<PathBuf>,
  store: Option<PathBuf>,
  receive: &mpsc::Receiver<Input>,
  latest: &Mutex<Option<Frame>>,
  repaint: &impl Fn(),
  wake: &Wake,
  pending: &AtomicBool,
) -> Result<()> {
  let mut session = Session::new(store);
  session.frontend = crate::raster::Frontend::Native(wake.clone());
  let mut screen = Screen {
    columns: 100,
    rows: 40,
    cell_width: 10,
    cell_height: 20,
    origin: (0, 0),
  };
  for path in paths {
    apply(&mut session, Input::Open(path), &mut screen);
  }
  if session.tabs.is_empty() {
    session.home.get_or_insert_with(|| {
      Home::with_wake(&session.state.recent, screen, &session.frontend.wake())
    });
  }
  let mut cache = Cache::new(BUDGET);
  let mut chores = super::run::Chores::new(Instant::now());
  let mut dirty = true;
  loop {
    if let Some(home) = &mut session.home {
      dirty |= home.updates();
    }
    if !session.tabs.is_empty() {
      dirty |= session.hover_preview(Instant::now());
      dirty |= session.autoscroll(Instant::now());
      dirty |= completions(&mut session, &mut cache);
      dirty |= chores.run(&mut session, &mut std::io::sink())?;
      for app in &mut session.tab().panes {
        dirty |= app.animate();
        dirty |= app.reader.expire_flash(Instant::now());
        dirty |= app.bar_expired(Instant::now());
      }
    }
    if dirty {
      let frame = frame(&mut session, &mut cache, screen);
      match frame {
        Ok(frame) => publish(latest, frame),
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
    let wait = if session.tabs.is_empty() {
      Duration::from_secs(2)
    } else {
      let animated =
        session.tab().panes.iter().any(|app| app.reader.animation().active());
      let wait = chores.wait(&mut session);
      if animated { wait.min(Duration::from_millis(8)) } else { wait }
    };
    let input = match receive.recv_timeout(wait) {
      Ok(input) => input,
      Err(mpsc::RecvTimeoutError::Timeout) => continue,
      Err(mpsc::RecvTimeoutError::Disconnected) => break,
    };
    if matches!(input, Input::Close) {
      break;
    }
    if matches!(input, Input::Wake) {
      pending.store(false, Ordering::Release);
      continue;
    }
    if matches!(input, Input::Presented) {
      session.drawn();
      continue;
    }
    chores.touch();
    apply(&mut session, input, &mut screen);
    dirty = true;
  }
  session.finish()
}

fn publish(latest: &Mutex<Option<Frame>>, mut frame: Frame) {
  let Ok(mut output) = latest.lock() else { return };
  if frame.clipboard.is_none() {
    frame.clipboard =
      output.as_mut().and_then(|previous| previous.clipboard.take());
  }
  *output = Some(frame);
}

fn apply(session: &mut Session, input: Input, screen: &mut Screen) {
  let applied = match input {
    Input::Resize(size) => {
      *screen = size;
      session.resize(size)
    }
    Input::Open(path) => {
      session.open(&path, true, *screen).map(|()| session.home = None)
    }
    Input::Key(key) if session.tabs.is_empty() => {
      home_event(session, vpdf_terminal::decode::Event::Key(key), *screen)
    }
    Input::Paste(text) if session.tabs.is_empty() => {
      home_event(session, vpdf_terminal::decode::Event::Paste(text), *screen)
    }
    Input::Key(key) => session.type_key(key).and_then(|()| session.settle()),
    Input::Paste(text) => {
      let active = session.active;
      let tab = &mut session.tabs[active];
      tab.panes[tab.focus]
        .paste(&text, &mut session.history)
        .and_then(|()| session.settle())
    }
    Input::Mouse { event, pixels: (x, y), screen: drawn }
      if drawn == *screen =>
    {
      let mut report = vpdf_terminal::decode::Mouse::from(event);
      report.at = vpdf_terminal::decode::Spot::Pixel { x, y };
      if session.tabs.is_empty() {
        home_event(
          session,
          vpdf_terminal::decode::Event::Mouse(report),
          *screen,
        )
      } else {
        session.mouse_report(report).and_then(|_| session.settle())
      }
    }
    Input::History { forward, pixels: (x, y), screen: drawn }
      if drawn == *screen && !session.tabs.is_empty() =>
    {
      use vpdf_terminal::decode::{Button, Mouse, MouseKind, Spot};
      let button = if forward { Button::FORWARD } else { Button::BACK };
      let report = Mouse {
        kind: MouseKind::Press(button),
        at: Spot::Pixel { x, y },
        modifiers: crossterm::event::KeyModifiers::NONE,
      };
      session.mouse_report(report).and_then(|_| session.settle())
    }
    Input::Leave => {
      session.pointer_left();
      session.drop_pointer();
      Ok(())
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
  if session.tabs.is_empty() && session.home.is_none() && !session.quit {
    session.home = Some(Home::with_wake(
      &session.state.recent,
      *screen,
      &session.frontend.wake(),
    ));
  }
  if let Err(error) = applied {
    if let Some(home) = &mut session.home {
      home.view.fail(error.to_string());
    } else if !session.tabs.is_empty() {
      session.app().error = Some(error.to_string());
    }
  }
}

fn home_event(
  session: &mut Session,
  event: vpdf_terminal::decode::Event,
  screen: Screen,
) -> Result<()> {
  let Some(home) = &mut session.home else { return Ok(()) };
  match home.event(event, &mut session.history)? {
    crate::home::Outcome::Open(path) => {
      session.open(&path, true, screen)?;
      session.home = None;
    }
    crate::home::Outcome::Quit => session.quit = true,
    _ => {}
  }
  Ok(())
}

fn completions(
  session: &mut Session,
  cache: &mut Cache<TileId, Arc<Vec<u8>>>,
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
    "vpdf".to_owned()
  } else {
    session.prepare_frame();
    let area = session.area();
    let tab = session.tab();
    for index in 0..tab.panes.len() {
      if index != tab.focus {
        let app = &mut tab.panes[index];
        app.scene_pane(&mut recorder, false)?;
        complete &= app.complete || app.covered();
      }
    }
    tab.tree.scene_dividers(&mut recorder, area);
    let app = &mut tab.panes[tab.focus];
    app.scene_pane(&mut recorder, false)?;
    complete &= app.complete || app.covered();
    let title = app.title();
    if let Some(notice) = &session.unmeasured {
      notice.scene(&mut recorder);
    }
    if session.state.perf {
      session.perf.view.scene(&mut recorder, screen, true);
    }
    session.finish_frame();
    title
  };
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
    title,
    cursor,
    complete,
    quit: session.quit,
    clipboard,
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crossterm::event::{KeyCode, KeyModifiers};

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

  #[test]
  fn opens_renders_navigates_and_saves() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("native.pdf");
    crate::fixtures::pdf(&path, &["first", "second"], "")?;
    let store = root.path().join("state.json");
    let (send, wakes) = mpsc::channel();
    let mut bridge =
      Bridge::stored(vec![path.clone()], Some(store.clone()), move || {
        let _ = send.send(());
      })?;
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
    let mut home = Bridge::stored(Vec::new(), Some(store), move || {
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
      let mut streaming = Graphics::default();
      let mut retained = Graphics::default();
      for _ in 0..2 {
        let mut expected = Vec::new();
        app.blank = false;
        streaming.begin(&mut expected)?;
        app.draw_pane(&mut streaming, &mut expected)?;
        streaming.end(&mut expected)?;
        app.blank = false;
        let mut recorder = Recorder::new(|_| false);
        app.scene_pane(&mut recorder, streaming.crop_quirk())?;
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
    use crossterm::event::{MouseButton, MouseEventKind};
    use vpdf_model::document::Target;
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
    use crossterm::event::{MouseButton, MouseEventKind};
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
    use crossterm::event::MouseEventKind;
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
    use crossterm::event::{MouseButton, MouseEventKind};
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
      completions(&mut session, &mut cache);
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
    assert!(completions(&mut session, &mut cache));
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
    let mut bridge = Bridge::stored(Vec::new(), None, move || {
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
