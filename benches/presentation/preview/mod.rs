use crate::presentation;
use anyhow::Result;
use std::hint::black_box;
use vpdf_model::geometry::Screen;
use vpdf_model::tile::TileKey;
use vpdf_ui::layout::Layout;
use vpdf_ui::source::Raster;

#[derive(serde::Deserialize)]
struct Scene {
  text: bool,
  target: vpdf_model::document::Target,
  source: vpdf_model::text::Position,
  cursor: Option<[f32; 4]>,
  screen: [u16; 4],
  scroll: f32,
  rasters: Vec<Ready>,
  output_sha256: [String; 2],
}

#[derive(serde::Deserialize)]
struct Ready {
  page: vpdf_model::text::PageId,
  scale: u32,
  rotation: u16,
  rect: [i32; 4],
  crop: vpdf_model::geometry::Crop,
  pixels: Vec<u8>,
  stride: usize,
}

impl Ready {
  fn key(&self) -> TileKey {
    let [x, y, width, height] = self.rect;
    TileKey {
      page: self.page,
      scale: self.scale,
      rotation: self.rotation,
      x,
      y,
      width,
      height,
      crop: self.crop,
    }
  }
}

struct Source<'a> {
  pages: &'a super::fixtures::TextPages,
  rasters: Vec<Ready>,
}

impl vpdf_reader::reader::TextSource for Source<'_> {
  fn page_count(&self) -> usize {
    vpdf_reader::reader::TextSource::page_count(&self.pages)
  }
  fn page_text(
    &mut self,
    page: vpdf_model::text::PageId,
  ) -> Result<std::sync::Arc<vpdf_model::text::Text>> {
    vpdf_reader::reader::TextSource::page_text(&mut self.pages, page)
  }
}

impl Raster for Source<'_> {
  fn raster(
    &mut self,
    key: TileKey,
    visit: impl FnOnce(&[u8], usize) -> Result<()>,
  ) -> Result<()> {
    let ready = self
      .rasters
      .iter()
      .find(|ready| ready.key() == key)
      .ok_or_else(|| anyhow::anyhow!("Missing preview raster"))?;
    visit(&ready.pixels, ready.stride)
  }
}

pub fn draw() -> Result<()> {
  use vpdf_model::geometry::At;
  use vpdf_model::text::Position;
  use vpdf_terminal::graphics::Graphics;
  use vpdf_ui::preview::{Frame, Preview};
  let cases: Vec<Scene> = super::fixture("preview")?;
  let pages = super::fixtures::text()?;
  for case in cases {
    let [columns, rows, cell_width, cell_height] = case.screen;
    let screen =
      Screen { columns, rows, cell_width, cell_height, origin: (0, 0) };
    let layout = Layout::new(pages.metadata.iter().map(|p| p.1), screen);
    let mut source = Source { pages: &pages, rasters: case.rasters };
    let mut preview = Preview::open(
      case.target,
      case.source,
      true,
      case.text,
      &layout,
      &mut source,
    )?;
    super::measure(&format!("Preview::input text={}", case.text), || {
      use vpdf_ui::keys::{Action, Input};
      for action in [Action::Scroll(0.5), Action::Tab(true), Action::Zoom(1)] {
        let input = Input { action, count: 3, explicit: true, register: 'a' };
        black_box(Preview::input(black_box(input), &layout));
      }
    })?;
    let mut graphics = Graphics::default();
    let mut out = Vec::new();
    let frame = || Frame {
      layout: &layout,
      pane: vpdf_ui::paint::Frame {
        at: At::default(),
        screen,
        left: 0,
        text: case.text,
      },
      identity: (case.scroll.to_bits(), Position::default()),
      cursor: case.cursor.map(super::bounds),
      link: None,
    };
    anyhow::ensure!(
      preview.scroll(0.0, &layout, &mut source)?,
      "Invalid prepared preview"
    );
    let mut recorder = vpdf_model::scene::Recorder::new(|_| false);
    preview.scene(&mut recorder, frame(), &&pages).unwrap();
    let scene = recorder.into_scene();
    let mut replay = Graphics::default();
    let mut draw = |out: &mut Vec<u8>| -> Result<()> {
      out.clear();
      anyhow::ensure!(
        preview.scroll(0.0, &layout, &mut source)?,
        "Invalid prepared preview"
      );
      graphics.begin(out)?;
      presentation::draw(out, Some(&mut graphics), |scene| {
        preview.scene(scene, frame(), &&pages)
      })??;
      graphics.end(out)?;
      Ok(())
    };
    for hash in case.output_sha256 {
      draw(&mut out)?;
      let mut actual = Vec::new();
      replay.begin(&mut actual)?;
      let mut painter = vpdf_terminal::painter::Painter::new(&mut actual)
        .with_graphics(&mut replay);
      painter.paint(&scene);
      painter.finish()?;
      replay.end(&mut actual)?;
      anyhow::ensure!(actual == out, "Retained preview differs");
      anyhow::ensure!(
        super::hash::digest(&out) == hash,
        "Preview output differs: text={}",
        case.text
      );
    }
    super::measure(&format!("Preview::draw text={}", case.text), || {
      draw(&mut out).expect("prepared preview");
      black_box(&out);
    })?;
    super::measure(&format!("Preview::keep text={}", case.text), || {
      black_box(preview.keep(black_box(true)));
    })?;
  }
  Ok(())
}
