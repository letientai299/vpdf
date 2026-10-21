use anyhow::Result;
use std::hint::black_box;
use vpdf_model::document::Target;
use vpdf_model::geometry::At;
use vpdf_model::geometry::Screen;
use vpdf_model::text::{PageId, Position};
use vpdf_reader::reader::Reader;
use vpdf_ui::layout::{Layout, Zoom};
use vpdf_ui::paint;
use vpdf_ui::reading::{Frame, Pane, Sections, Tracking};
use vpdf_ui::text_view::TextView;

#[derive(serde::Deserialize)]
struct Prepared {
  headings: Vec<(String, usize, Target)>,
}

pub fn select() -> Result<()> {
  let prepared = super::fixtures::text()?;
  let outline: Prepared = super::fixture("scrollbar")?;
  let targets: Vec<_> =
    outline.headings.into_iter().map(|(_, _, t)| t).collect();
  anyhow::ensure!(!targets.is_empty(), "Missing benchmark sections");
  for text in [false, true] {
    for cursor in [false, true] {
      case(&prepared, &targets, text, cursor)?;
    }
  }
  Ok(())
}

fn case(
  prepared: &super::fixtures::TextPages,
  targets: &[Target],
  text: bool,
  cursor: bool,
) -> Result<()> {
  let screen = Screen {
    columns: 100,
    rows: 30,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let bounds = || prepared.metadata.iter().map(|(_, bounds)| *bounds);
  let mut layout = Layout::new(bounds(), screen);
  layout.request_zoom(Zoom::To(2.0));
  layout.reflow(bounds());
  let mut reader = Reader::default();
  reader.cursor = Position { page: PageId(0), line: 10, glyph: 4 };
  anyhow::ensure!(
    prepared.text[0].glyph(reader.cursor).is_some(),
    "Missing benchmark cursor"
  );
  if cursor {
    reader.enter_cursor();
  }
  let mut pane = Pane::new(reader, Tracking::default());
  let mut text_view = TextView::default();
  let frame = Frame {
    pane: paint::Frame { screen, at: At::default(), left: 0, text },
    visible: screen,
  };
  let mut view = pane.bind(&mut layout, &mut text_view, frame, prepared);
  let sections = || Sections::new(targets.iter(), prepared.metadata.len());
  let expected = view.section(sections(), true, 1)?;
  anyhow::ensure!(expected.is_some(), "Missing next section");
  super::measure(
    &format!("Reading::section text={text} cursor={cursor}"),
    || {
      let section = view
        .section(sections(), true, black_box(1))
        .expect("prepared headings");
      black_box(section.map(|section| section.target()));
    },
  )
}
