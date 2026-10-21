use anyhow::Result;
use std::hint::black_box;
use vpdf_model::geometry::At;
use vpdf_model::geometry::Screen;
use vpdf_model::text::PageId;
use vpdf_reader::reader::Reader;
use vpdf_search::{
  query::Query,
  search::{Index, compile},
};
use vpdf_ui::find::Search;
use vpdf_ui::layout::Layout;
use vpdf_ui::paint;
use vpdf_ui::reading::{Frame, Pane, Tracking};
use vpdf_ui::text_view::TextView;

pub fn navigate() -> Result<()> {
  let prepared = super::fixtures::text()?;
  let query = Query { text: "attention".into(), ..Query::default() };
  let pattern = compile(&query.text, false)?;
  let mut hits = Vec::new();
  for (page, text) in prepared.text.iter().enumerate() {
    hits.extend(Index::new(PageId(page), text).matches(&pattern, 10_000)?);
  }
  anyhow::ensure!(hits.len() >= 2, "Missing search targets");
  let start = hits[0].first;
  let screen = Screen {
    columns: 100,
    rows: 30,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  for text in [false, true] {
    let mut layout =
      Layout::new(prepared.metadata.iter().map(|(_, b)| *b), screen);
    let mut reader = Reader::default();
    reader.enter_cursor();
    let mut pane = Pane::new(reader, Tracking::default());
    let mut text_view = TextView::default();
    let frame = Frame {
      pane: paint::Frame { screen, at: At::default(), left: 0, text },
      visible: screen,
    };
    let mut search = Search::default();
    search.show(query.clone(), hits.clone());
    let mut view = pane.bind(&mut layout, &mut text_view, frame, &prepared);
    view.land_at(start)?;
    view.next_result(&mut search, false, 1)?;
    view.next_result(&mut search, true, 1)?;
    super::measure(&format!("Reading::search text={text}"), || {
      for reverse in [false, true] {
        view
          .next_result(&mut search, black_box(reverse), 1)
          .expect("prepared hits");
      }
      black_box(search.hidden());
    })?;
    anyhow::ensure!(view.position()? == start, "Unexpected search target");
    anyhow::ensure!(view.word_query()?.is_some(), "Missing search word");
    super::measure(&format!("Reading::word_query text={text}"), || {
      black_box(view.word_query().expect("prepared word"));
    })?;
  }
  Ok(())
}
