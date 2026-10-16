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
  pub search: crate::find::Search,
  pub prompt: Option<crate::prompt::Prompt>,
  pub panel: Option<crate::panel::Panel>,
  pub preview: Option<bool>,
  pub ocr: Option<crate::ocr::OcrJob>,
  pub reset_images: bool,
  pub animation: Option<crate::animation::Scroll>,
  pub(crate) requested: Vec<TileKey>,
  pub(crate) last_click: Option<(Instant, Position)>,
}

impl App {
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
      search: crate::find::Search::default(),
      prompt: None,
      panel: None,
      preview: None,
      ocr: None,
      reset_images: false,
      animation: None,
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
  let _terminal = Terminal::enter()?;
  let mut secret = password.to_owned();
  let mut app = loop {
    match App::new(path, &secret, text) {
      Ok(app) => break app,
      Err(error) if error.to_string() == "PDF password required" => {
        secret = crate::prompt::password()?;
      }
      Err(error) => return Err(error),
    }
  };
  app.reader.edge(
    PageId(page.saturating_sub(1).min(app.doc.pages.len() - 1)),
    false,
    &mut app.doc,
  )?;
  app.ensure_cursor()?;
  let mut out = io::BufWriter::new(io::stdout());
  let mut trace = crate::trace::Trace::new()?;
  let mut buffer = Vec::with_capacity(64 * 1024);
  let mut dirty = true;
  let mut last_frame = Instant::now() - Duration::from_millis(9);
  while !app.quit {
    dirty |= app.raster_updates(&mut out)?;
    dirty |= app.search_updates()?;
    dirty |= app.ocr_updates()?;
    app.clipboard(&mut out)?;
    let frame_due = last_frame.elapsed() >= Duration::from_micros(8333);
    if frame_due {
      dirty |= app.animate();
      if dirty {
        trace.draw(&mut app, &mut out, &mut buffer)?;
        dirty = false;
        last_frame = Instant::now();
      }
    }
    let wait = if !dirty && app.animation.is_none() {
      Duration::from_millis(8)
    } else {
      Duration::from_micros(8333)
        .saturating_sub(last_frame.elapsed())
        .max(Duration::from_millis(1))
    };
    if !event::poll(wait)? {
      continue;
    }
    trace.input();
    let result = match event::read()? {
      Event::Key(key) if key.kind != KeyEventKind::Release => app.key(key),
      Event::Paste(text) => {
        if let Some(prompt) = &mut app.prompt {
          prompt.text.extend(
            text
              .chars()
              .filter(|c| !c.is_control())
              .take(4096usize.saturating_sub(prompt.text.len())),
          );
        }
        Ok(())
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
