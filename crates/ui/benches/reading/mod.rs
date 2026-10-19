use anyhow::Result;
use std::hint::black_box;
use vpdf_model::geometry::Screen;
use vpdf_model::text::{PageId, Position};
use vpdf_reader::motion::Motion;
use vpdf_reader::reader::Reader;
use vpdf_terminal::terminal::At;
use vpdf_ui::find::Search;
use vpdf_ui::keys::{Action, Input};
use vpdf_ui::layout::Layout;
use vpdf_ui::paint;
use vpdf_ui::reading::{Frame, Pane, ScrollInput, Tracking};
use vpdf_ui::text_view::TextView;

pub fn scroll() -> Result<()> {
  run(false)
}

pub fn navigation() -> Result<()> {
  run(true)
}

fn run(navigate: bool) -> Result<()> {
  let prepared = super::fixtures::text()?;
  let screen = Screen {
    columns: 100,
    rows: 30,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  for text in [false, true] {
    for cursor in [false, true] {
      case(&prepared, screen, text, cursor, navigate)?;
    }
  }
  Ok(())
}

fn case(
  prepared: &super::fixtures::TextPages,
  screen: Screen,
  text: bool,
  cursor: bool,
  navigate: bool,
) -> Result<()> {
  let bounds = || prepared.metadata.iter().map(|(_, bounds)| *bounds);
  let mut layout = Layout::new(bounds(), screen);
  layout.request_zoom(vpdf_ui::layout::Zoom::To(2.0));
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
  let mut search = Search::default();
  let operation = if navigate { "navigate" } else { "scroll" };
  super::measure(
    &format!("Reading::{operation} text={text} cursor={cursor}"),
    || {
      for delta in [16.0, -16.0] {
        let mut view = pane.bind(&mut layout, &mut text_view, frame, prepared);
        if navigate {
          let motion = if delta > 0.0 { Motion::Down } else { Motion::Up };
          let input = Input {
            action: Action::Motion(motion),
            count: 1,
            explicit: false,
            register: '"',
          };
          black_box(
            view
              .navigate(black_box(input), &mut search)
              .expect("prepared navigation"),
          );
        } else {
          view
            .scroll(black_box(delta), ScrollInput::Wheel)
            .expect("prepared scroll");
        }
      }
      black_box((
        pane.cursor,
        pane.scroll,
        pane.target(),
        pane.animation().target(),
        text_view.top(),
      ));
    },
  )?;
  Ok(())
}
