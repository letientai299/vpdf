use super::*;
use crate::text::{Glyph, Text};
use std::sync::Arc;
use vpdf_reader::reader::Reader;
use vpdf_search::search::Hit;

pub(super) struct Source {
  pub(super) text: Arc<Text>,
  pub(super) reads: Vec<PageId>,
  pub(super) fail: bool,
}

impl Source {
  pub(super) fn new() -> Self {
    Self {
      text: Arc::new(Text {
        truncated: false,
        lines: vec![Line {
          paragraph: false,
          glyphs: vec![Glyph {
            ch: 'a',
            bounds: Bounds { x0: 10.0, y0: 10.0, x1: 20.0, y1: 20.0 },
          }],
        }],
      }),
      reads: Vec::new(),
      fail: false,
    }
  }
}

impl Pages for Source {
  fn count(&self) -> usize {
    3
  }
  fn bounds(&self, _: PageId) -> Bounds {
    Bounds { x0: 0.0, y0: 0.0, x1: 100.0, y1: 1000.0 }
  }
  fn label(&self, _: PageId) -> &'static str {
    ""
  }
}

impl TextSource for Source {
  fn page_count(&self) -> usize {
    self.count()
  }
  fn page_text(&mut self, page: PageId) -> Result<Arc<Text>> {
    self.reads.push(page);
    anyhow::ensure!(!self.fail, "Text unavailable");
    Ok(Arc::clone(&self.text))
  }
}

pub(super) fn layout(source: &Source) -> Layout {
  Layout::new(
    (0..source.count()).map(|p| source.bounds(PageId(p))),
    Screen {
      columns: 10,
      rows: 10,
      cell_width: 10,
      cell_height: 10,
      origin: (0, 0),
    },
  )
}

fn marks(reader: &Reader) -> Marks<'_> {
  Marks {
    reader,
    matches: &[],
    flash: None,
    landing: None,
    cursorline: true,
    dim: false,
    hovered: &[],
  }
}

#[test]
fn text_requests_follow_visible_matches() -> Result<()> {
  let mut source = Source::new();
  let layout = layout(&source);
  let mut reader = Reader::default();
  reader.scroll = layout.pages()[1].top;
  let hits: Vec<_> = [0, 1, 2]
    .into_iter()
    .flat_map(|page| {
      let position = Position { page: PageId(page), ..Position::default() };
      std::iter::repeat_n(
        Hit { first: position, last: position, excerpt: String::new() },
        301,
      )
    })
    .collect();
  let marks = Marks { matches: &hits, ..marks(&reader) };
  let canvas =
    Canvas { layout: &layout, screen: layout.screen(), left: 0, menu: None };
  canvas.draw(
    &mut source,
    &marks,
    &mut Graphics::default(),
    &mut Vec::new(),
  )?;
  assert_eq!(source.reads, vec![PageId(1); 300]);
  Ok(())
}

#[test]
fn idle_and_hover_need_no_text() -> Result<()> {
  let mut source = Source::new();
  source.fail = true;
  let layout = layout(&source);
  let reader = Reader::default();
  let hovered = [(PageId(0), source.text.lines[0].glyphs[0].bounds)];
  let canvas =
    Canvas { layout: &layout, screen: layout.screen(), left: 0, menu: None };
  for hovered in [&[][..], &hovered[..]] {
    let marks = Marks { hovered, ..marks(&reader) };
    canvas.draw(
      &mut source,
      &marks,
      &mut Graphics::default(),
      &mut Vec::new(),
    )?;
  }
  assert_eq!(source.reads, [] as [PageId; 0]);
  Ok(())
}

#[test]
fn text_errors_stop_drawing() {
  let mut source = Source::new();
  source.fail = true;
  let layout = layout(&source);
  let mut reader = Reader::default();
  reader.visual(SelectionKind::Character);
  let canvas =
    Canvas { layout: &layout, screen: layout.screen(), left: 0, menu: None };
  let mut out = Vec::new();
  let error = canvas
    .draw(&mut source, &marks(&reader), &mut Graphics::default(), &mut out)
    .unwrap_err();
  assert_eq!(error.to_string(), "Text unavailable");
  assert_eq!(source.reads, [PageId(0)]);
  assert_eq!(out, [] as [u8; 0]);
}
