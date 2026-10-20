//! First-use interactions with isolated native lifetimes.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::pdf_text::PdfText;
use vpdf::solo::Solo;
use vpdf_model::document::Target;
use vpdf_model::geometry::{Crop, page_matrix, transform};
use vpdf_model::text::PageId;
use vpdf_model::tile::{TILE_HEIGHT, TILE_WIDTH, TileKey};
use vpdf_ui::layout::Screen;
use vpdf_ui::pointer::{Hit, Pixel, Region};

pub const PANE: Screen = Screen {
  columns: 120,
  rows: 40,
  cell_width: 8,
  cell_height: 16,
  origin: (0, 0),
};

#[derive(
  Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Interaction {
  Selection,
  Cursor,
  Hover,
  Figures,
  Preview,
}

impl std::str::FromStr for Interaction {
  type Err = String;
  fn from_str(name: &str) -> std::result::Result<Self, String> {
    match name {
      "selection" => Ok(Self::Selection),
      "cursor" => Ok(Self::Cursor),
      "hover" => Ok(Self::Hover),
      "figures" => Ok(Self::Figures),
      "preview" => Ok(Self::Preview),
      _ => Err(format!("Unknown interaction: {name}")),
    }
  }
}

#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Preparation {
  Cold,
  List,
  Text,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct Request {
  pub path: PathBuf,
  pub name: Interaction,
  pub page: usize,
  pub prepared: Preparation,
  pub rendering: bool,
}

pub fn plan(pages: usize, samples: usize) -> Vec<usize> {
  fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
      (a, b) = (b, a % b);
    }
    a
  }
  if pages == 0 {
    return Vec::new();
  }
  let mut stride = 17;
  while gcd(stride, pages) != 1 {
    stride += 1;
  }
  (0..samples).map(|i| (pages / 3 + i * stride) % pages).collect()
}

struct Scene {
  page: PageId,
  destination: Option<PageId>,
  hover: Option<(Pixel, usize)>,
  target: Option<Target>,
}

fn key(app: &mut Solo, ch: char) -> Result<()> {
  app.key(
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
    &mut vpdf_ui::prompt::History::default(),
  )
}

fn hovered(app: &mut Solo, pixel: Pixel) -> Result<Hit> {
  app.hovered(
    Region::Page,
    pixel.x / pixel.cell.0,
    pixel.y / pixel.cell.1,
    Some(pixel),
  )
}

fn act(app: &mut Solo, name: Interaction, scene: &Scene) -> Result<()> {
  match name {
    Interaction::Selection => {
      key(app, 'v')?;
      anyhow::ensure!(app.reader.selection.is_some(), "Selection missing");
    }
    Interaction::Cursor => {
      key(app, 'i')?;
      let before = app.reader.cursor;
      key(app, 'l')?;
      anyhow::ensure!(
        app.reader.uses_cursor() && app.reader.cursor != before,
        "Cursor unchanged"
      );
    }
    Interaction::Hover => {
      let (pixel, index) = scene.hover.expect("link geometry");
      anyhow::ensure!(
        hovered(app, pixel)? == Hit::Link(scene.page, index),
        "Link hit missing"
      );
    }
    Interaction::Figures => {
      black_box(app.doc.figures(scene.page)?);
    }
    Interaction::Preview => {
      app.preview_link()?;
      anyhow::ensure!(
        app
          .overlays
          .preview()
          .is_some_and(|peek| Some(peek.target()) == scene.target.as_ref()),
        "Preview target differs"
      );
    }
  }
  Ok(())
}

fn scene(app: &mut Solo, request: &Request) -> Result<Option<Scene>> {
  anyhow::ensure!(
    request.page < app.doc.pages.len(),
    "Sample page outside document"
  );
  let page = PageId(request.page);
  let scroll = app.layout.pages()[page.0].top;
  app.reading_view().scrub_scroll(scroll);
  let mut scene = Scene { page, destination: None, hover: None, target: None };
  if matches!(request.name, Interaction::Selection | Interaction::Cursor)
    && !app.reading_view().enter_text()?
  {
    return Ok(None);
  }
  if request.name == Interaction::Cursor {
    let mut reader = app.reader.clone();
    let old = reader.cursor;
    reader.step(1, false, &mut PdfText(&mut app.doc))?;
    if reader.cursor == old {
      return Ok(None);
    }
  }
  if matches!(request.name, Interaction::Preview | Interaction::Hover) {
    let link = app
      .doc
      .links(page)?
      .iter()
      .enumerate()
      .find(|(_, link)| {
        request.name == Interaction::Hover
          || !matches!(link.target, Target::Uri(_))
      })
      .map(|(index, link)| (index, link.clone()));
    let Some((index, link)) = link else { return Ok(None) };
    let (x, y) = link.first().centre();
    let position = app.doc.text(page)?.nearest(page, x, y);
    let rect = app.layout.project(page, link.first(), 0.0);
    let scroll = rect.centre().1 - PANE.height() / 2.0;
    app.reading_view().scrub_scroll(scroll);
    app.reader.point_to(position);
    if request.name == Interaction::Hover {
      let rect = app.layout.project(page, link.first(), app.reader.scroll);
      let (x, y) = rect.centre();
      if x < 0.0 || y < 0.0 || x >= PANE.width() || y >= PANE.height() {
        return Ok(None);
      }
      let pixel = Pixel {
        x: x as u16,
        y: y as u16,
        cell: (PANE.cell_width, PANE.cell_height),
      };
      if hovered(app, pixel)? != Hit::Link(page, index) {
        return Ok(None);
      }
      scene.hover = Some((pixel, index));
    } else {
      if app.link_target(position)? != Some(link.target.clone()) {
        return Ok(None);
      }
      scene.target = Some(link.target.clone());
      scene.destination = match link.target {
        Target::Page(page) | Target::Point { page, .. } => Some(page),
        Target::Uri(_) => None,
      }
      .filter(|destination| *destination != page);
    }
  }
  Ok(Some(scene))
}

fn ready(app: &mut Solo) -> Result<()> {
  let bounds = app.doc.pages[0].bounds;
  let rect = transform(bounds, &page_matrix(bounds, 1, 0));
  let (width, height) =
    (rect.width().ceil() as i32, rect.height().ceil() as i32);
  anyhow::ensure!(
    width <= TILE_WIDTH && height <= TILE_HEIGHT,
    "Readiness page exceeds one tile"
  );
  let key = TileKey {
    page: PageId(0),
    scale: 1,
    rotation: 0,
    x: 0,
    y: 0,
    width,
    height,
    crop: Crop::default(),
  };
  app.worker.service.request(&[key], 1);
  let tile = app
    .worker
    .results
    .recv_timeout(Duration::from_secs(10))?
    .map_err(anyhow::Error::msg)?;
  anyhow::ensure!(tile.key == key, "Readiness tile differs");
  Ok(())
}

fn settle(app: &mut Solo, frame: &mut Vec<u8>) -> Result<()> {
  let started = Instant::now();
  loop {
    app.raster_updates();
    app.animate();
    frame.clear();
    app.draw(frame)?;
    if app.complete {
      return Ok(());
    }
    anyhow::ensure!(
      started.elapsed() < Duration::from_secs(10),
      "Render timeout"
    );
    std::thread::sleep(Duration::from_millis(1));
  }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub struct Observation {
  pub page: usize,
  pub destination: Option<usize>,
  pub milliseconds: Option<f64>,
  pub preparation_ms: Option<f64>,
  pub text_cache_bytes: usize,
  pub ready_ms: Option<f64>,
}

impl Observation {
  pub fn skipped(page: usize) -> Self {
    Self {
      page,
      destination: None,
      milliseconds: None,
      preparation_ms: None,
      text_cache_bytes: 0,
      ready_ms: None,
    }
  }
}

pub fn sample(request: &Request) -> Result<Observation> {
  let mut app = Solo::open(&request.path, "", false, PANE)?;
  let Some(scene) = scene(&mut app, request)? else {
    return Ok(Observation::skipped(request.page));
  };
  ready(&mut app)?;
  app.doc.trim();
  let started = Instant::now();
  for page in [Some(scene.page), scene.destination].into_iter().flatten() {
    match request.prepared {
      Preparation::List => {
        black_box(app.doc.display(page)?);
      }
      Preparation::Text => {
        black_box(app.doc.text(page)?);
      }
      Preparation::Cold => {}
    }
  }
  let preparation_ms = started.elapsed().as_secs_f64() * 1000.0;
  let text_cache_bytes = app.doc.cache_bytes();
  let mut frame = Vec::new();
  if request.rendering {
    app.draw(&mut frame)?;
    anyhow::ensure!(!app.complete, "Rendering workload missing");
    anyhow::ensure!(
      app.doc.cache_bytes() == text_cache_bytes,
      "Frame warmed text"
    );
  }
  let started = Instant::now();
  act(&mut app, request.name, &scene)?;
  let milliseconds = started.elapsed().as_secs_f64() * 1000.0;
  let ready_ms = if request.rendering {
    settle(&mut app, &mut frame)?;
    Some(started.elapsed().as_secs_f64() * 1000.0)
  } else {
    None
  };
  Ok(Observation {
    page: scene.page.0,
    destination: scene.destination.map(|page| page.0),
    milliseconds: Some(milliseconds),
    preparation_ms: Some(preparation_ms),
    text_cache_bytes,
    ready_ms,
  })
}
