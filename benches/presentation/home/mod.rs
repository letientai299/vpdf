use crate::presentation;
use anyhow::Result;
use std::hint::black_box;
use std::path::PathBuf;
use vpdf_model::document::{Facts, LoadedFacts, Thumbnail};
use vpdf_model::text::Position;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::home::Home;
use vpdf_ui::layout::Screen;
use vpdf_ui::panel::{Destination, Row, Style, recent::Entry};

#[derive(serde::Deserialize)]
struct Prepared {
  path: PathBuf,
  label: String,
  detail: String,
  fit: Option<(u32, u32)>,
  screen: [u16; 4],
  pages: usize,
  bytes: u64,
  metadata: Vec<(String, String)>,
  section: Option<String>,
  thumb: Pixels,
}

#[derive(serde::Deserialize)]
struct Pixels {
  width: i32,
  height: i32,
  rgb: Vec<u8>,
}

pub fn draw() -> Result<()> {
  let data: Prepared = super::fixture("home")?;
  let [columns, rows, cell_width, cell_height] = data.screen;
  let entry = Entry {
    path: data.path.clone(),
    cursor: Position::default(),
    rotation: 0,
    used: 0,
    marks: 0,
  };
  let row = Row {
    style: Style::Path,
    ..Row::new(data.label, data.detail, Destination::File(data.path.clone()))
  };
  let mut home = Home::new(
    vec![row],
    &[entry],
    Screen { columns, rows, cell_width, cell_height, origin: (0, 0) },
  );
  home.shelf_mut().expect("recent shelf").accept(
    data.path,
    LoadedFacts {
      fit: data.fit,
      facts: Ok(Facts {
        pages: data.pages,
        bytes: data.bytes,
        metadata: data.metadata,
        section: data.section,
        thumb: Some(
          Thumbnail::new(data.thumb.width, data.thumb.height, data.thumb.rgb)
            .expect("Valid fixture thumbnail"),
        ),
      }),
    },
  );
  let mut recorder = vpdf_model::scene::Recorder::new(|_| false);
  home.scene(&mut recorder, 0, |_| {}).unwrap();
  let scene = recorder.into_scene();
  let mut replay = Graphics::default();
  let mut graphics = Graphics::default();
  let mut output = Vec::new();
  let mut draw = |output: &mut Vec<u8>| -> Result<()> {
    output.clear();
    graphics.begin(output)?;
    presentation::draw(output, Some(&mut graphics), |scene| {
      home.scene(scene, 0, |_| {})
    })??;
    graphics.end(output)?;
    Ok(())
  };
  for first in [true, false] {
    draw(&mut output)?;
    let mut actual = Vec::new();
    replay.begin(&mut actual)?;
    let mut painter = vpdf_terminal::painter::Painter::new(&mut actual)
      .with_graphics(&mut replay);
    painter.paint(&scene);
    painter.finish()?;
    replay.end(&mut actual)?;
    anyhow::ensure!(actual == output, "Retained home differs");
    assert_eq!(output.windows(4).any(|bytes| bytes == b"a=t,"), first);
    println!("home output {}", super::hash::digest(&output));
  }
  super::measure("Home::draw warm", || {
    draw(&mut output).expect("prepared home");
    black_box(&output);
  })
}
