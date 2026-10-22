use anyhow::Result;
use std::sync::Arc;
use vpdf_model::geometry::Screen;
use vpdf_model::text::{Bounds, Glyph, Line, PageId, Text};
use vpdf_reader::reader::{Reader, TextSource};
use vpdf_ui::layout::Layout;
use vpdf_ui::paint::Marks;
use vpdf_ui::source::Pages;

pub(crate) struct Source {
  pub(crate) text: Arc<Text>,
  pub(crate) reads: Vec<PageId>,
  pub(crate) fail: bool,
}

impl Source {
  pub(crate) fn new() -> Self {
    Self {
      text: Arc::new(Text {
        truncated: false,
        lines: vec![Line {
          paragraph: false,
          glyphs: vec![Glyph {
            quad: None,
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

pub(crate) fn layout(source: &Source) -> Layout {
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

pub(crate) fn marks(reader: &Reader) -> Marks<'_> {
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

pub(crate) fn overlays()
-> Result<(Source, Layout, Reader, vpdf_model::scene::Scene<'static>)> {
  use vpdf_reader::reader::SelectionKind;
  let mut source = Source::new();
  let layout = layout(&source);
  let mut reader = Reader::default();
  reader.visual(SelectionKind::Character);
  let hovered = [(PageId(0), source.text.lines[0].glyphs[0].bounds)];
  let marks = Marks { hovered: &hovered, ..marks(&reader) };
  let canvas = vpdf_ui::page::Canvas {
    layout: &layout,
    screen: layout.screen(),
    left: 2,
    menu: None,
  };
  let mut scene = vpdf_model::scene::Scene::new();
  canvas.scene(&mut source, &marks, &mut scene)?;
  Ok((source, layout, reader, scene))
}
