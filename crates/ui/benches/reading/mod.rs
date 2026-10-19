use anyhow::Result;
use std::hint::black_box;
use vpdf_model::geometry::Screen;
use vpdf_model::text::{PageId, Position};
use vpdf_reader::reader::Reader;
use vpdf_terminal::terminal::At;
use vpdf_ui::layout::Layout;
use vpdf_ui::page::scroll::Scroll;
use vpdf_ui::paint;
use vpdf_ui::reading::{Frame, ScrollInput, View};
use vpdf_ui::text_view::TextView;

pub fn scroll() -> Result<()> {
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
      let mut target = None;
      let mut animation = Scroll::default();
      let mut text_view = TextView::default();
      let frame = Frame {
        pane: paint::Frame { screen, at: At::default(), left: 0, text },
        visible: screen,
      };
      super::measure(
        &format!("Reading::scroll text={text} cursor={cursor}"),
        || {
          for delta in [16.0, -16.0] {
            View::new(
              &mut reader,
              &mut target,
              &mut animation,
              &mut layout,
              &mut text_view,
              frame,
              &prepared,
            )
            .scroll(black_box(delta), ScrollInput::Wheel)
            .expect("prepared scroll");
          }
          black_box((
            reader.cursor,
            reader.scroll,
            target,
            animation.target(),
            text_view.top(),
          ));
        },
      )?;
    }
  }
  Ok(())
}
