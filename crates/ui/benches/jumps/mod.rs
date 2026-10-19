use anyhow::Result;
use std::hint::black_box;
use vpdf_model::geometry::Screen;
use vpdf_model::text::{PageId, Position};
use vpdf_reader::reader::Reader;
use vpdf_terminal::terminal::At;
use vpdf_ui::layout::{Layout, Zoom};
use vpdf_ui::paint;
use vpdf_ui::reading::{Frame, Pane, Tracking};
use vpdf_ui::text_view::TextView;

pub fn land() -> Result<()> {
  let prepared = super::fixtures::text()?;
  for text in [false, true] {
    for pages in [false, true] {
      case(&prepared, text, pages)?;
    }
  }
  Ok(())
}

fn case(
  prepared: &super::fixtures::TextPages,
  text: bool,
  pages: bool,
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
  let positions = if pages {
    [Position::default(), Position { page: PageId(1), ..Position::default() }]
  } else {
    [10, 20].map(|line| Position { page: PageId(0), line, glyph: 4 })
  };
  for position in positions {
    anyhow::ensure!(
      prepared.text[position.page.0].glyph(position).is_some(),
      "Missing benchmark target"
    );
  }
  let reader = Reader::default();
  let mut pane = Pane::new(reader, Tracking::default());
  let mut text_view = TextView::default();
  let frame = Frame {
    pane: paint::Frame { screen, at: At::default(), left: 0, text },
    visible: screen,
  };
  super::measure(&format!("Reading::jump text={text} pages={pages}"), || {
    for position in positions {
      let mut view = pane.bind(&mut layout, &mut text_view, frame, prepared);
      if pages {
        view.go_to_page(black_box(position.page)).expect("prepared page jump");
      } else {
        view.land_at(black_box(position)).expect("prepared landing");
      }
      black_box(pane.landing().position());
    }
  })?;
  anyhow::ensure!(
    pane.landing().position() == Some(positions[1]),
    "Unexpected landing target"
  );
  Ok(())
}
