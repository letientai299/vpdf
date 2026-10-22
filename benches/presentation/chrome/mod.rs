use crate::presentation;
use anyhow::Result;
use std::hint::black_box;
use std::path::PathBuf;
use vpdf_model::geometry::At;
use vpdf_model::geometry::Screen;
use vpdf_model::text::PageId;
use vpdf_reader::reader::Reader;
use vpdf_ui::chrome::{Bar, Document, Names, View};
use vpdf_ui::find::Search;
use vpdf_ui::input::InputOwner;
use vpdf_ui::keys::{Keys, PanelKind};
use vpdf_ui::layout::Layout;
use vpdf_ui::panel::{Panel, Row, help};

#[derive(serde::Deserialize)]
struct Prepared {
  path: PathBuf,
  screen: [u16; 4],
}

pub fn draw() -> Result<()> {
  let prepared: Prepared = super::fixture("home")?;
  let pages = super::fixtures::text()?;
  let [columns, rows, cell_width, cell_height] = prepared.screen;
  let screen =
    Screen { columns, rows, cell_width, cell_height, origin: (0, 0) };
  let width = Panel::width(PanelKind::Help, columns, None);
  let layout = Layout::new(
    pages.metadata.iter().map(|(_, bounds)| *bounds),
    Screen { columns: columns - width, ..screen },
  );
  let reader = Reader::default();
  let search = Search::default();
  let keys = Keys::default();
  let rows = help(|_| false).iter().map(|line| Row::parse(line)).collect();
  let panel = Panel::new(PanelKind::Help, rows);
  let file =
    prepared.path.file_name().expect("fixture filename").to_string_lossy();
  let bar = Bar {
    view: View {
      reader: &reader,
      layout: &layout,
      search: &search,
      keys: &keys,
      panel: Some(&panel),
      owner: InputOwner::Panel,
      capture: None,
      text: false,
      overview: false,
    },
    document: Document {
      page: PageId(0),
      pages: pages.metadata.len(),
      label: &pages.metadata[0].0,
      section: None,
    },
    names: Names { file: &file, names: &[], active: 0 },
    screen,
    error: None,
  };
  let mut out = Vec::new();
  presentation::try_draw(&mut out, None, |scene| {
    bar.scene(scene, At::default(), screen.rows - 1, true, None)
  })?;
  println!("status output {}", super::hash::digest(&out));
  super::measure("Bar::draw owner", || {
    out.clear();
    presentation::try_draw(&mut out, None, |scene| {
      bar.scene(scene, At::default(), screen.rows - 1, true, None)
    })
    .expect("owner badge");
    black_box(&out);
  })
}
