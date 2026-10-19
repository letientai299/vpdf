use super::*;
use crate::text::{Bounds, Glyph, Line, PageId, Text};
use std::sync::Arc;
use vpdf_reader::reader::{Operator, SelectionKind};
use vpdf_terminal::terminal::At;

struct Source {
  text: Arc<Text>,
  reads: usize,
  fail: usize,
}

impl Pages for Source {
  fn count(&self) -> usize {
    1
  }
  fn bounds(&self, _: PageId) -> Bounds {
    Bounds { x0: 0.0, y0: 0.0, x1: 100.0, y1: 1000.0 }
  }
  fn label(&self, _: PageId) -> &'static str {
    "1"
  }
}

impl TextSource for Source {
  fn page_count(&self) -> usize {
    1
  }
  fn page_text(&mut self, _: PageId) -> Result<Arc<Text>> {
    self.reads += 1;
    anyhow::ensure!(self.reads != self.fail, "Text unavailable");
    Ok(Arc::clone(&self.text))
  }
}

struct Fixture {
  reader: Reader,
  target: Option<Position>,
  animation: Scroll,
  layout: Layout,
  text: TextView,
  frame: Frame,
  source: Source,
}

impl Fixture {
  fn new() -> Self {
    let lines = (0..30)
      .map(|row| Line {
        paragraph: false,
        glyphs: vec![Glyph {
          ch: 'a',
          bounds: Bounds {
            x0: 10.0,
            x1: 20.0,
            y0: 10.0 + row as f32 * 20.0,
            y1: 20.0 + row as f32 * 20.0,
          },
        }],
      })
      .collect();
    let source = Source {
      text: Arc::new(Text { lines, truncated: false }),
      reads: 0,
      fail: 0,
    };
    let screen = Screen {
      columns: 10,
      rows: 10,
      cell_width: 10,
      cell_height: 10,
      origin: (0, 0),
    };
    let frame = Frame {
      pane: paint::Frame { screen, at: At::default(), left: 0, text: false },
      visible: screen,
    };
    Self {
      reader: Reader::default(),
      target: None,
      animation: Scroll::default(),
      layout: Layout::new([source.bounds(PageId(0))], screen),
      text: TextView::default(),
      frame,
      source,
    }
  }

  fn view(&mut self) -> View<'_, &mut Source> {
    View::new(
      &mut self.reader,
      &mut self.target,
      &mut self.animation,
      &mut self.layout,
      &mut self.text,
      self.frame,
      &mut self.source,
    )
  }
}

#[test]
fn scroll_failures_preserve_effect_order() {
  for (fail, scroll, target) in [(1, 200.0, None), (2, 240.0, Some(240.0))] {
    let mut fixture = Fixture::new();
    fixture.reader.enter_cursor();
    fixture.reader.scroll = 50.0;
    fixture.reader.desired = Some(9);
    fixture.reader.operator = Some(Operator { count: 1, register: '"' });
    fixture.reader.message = "pending".into();
    fixture.source.fail = fail;
    fixture.animation.start(200.0, Instant::now());
    assert!(fixture.view().scroll(40.0, ScrollInput::Wheel).is_err());
    assert_eq!(fixture.source.reads, fail);
    assert_eq!(fixture.reader.scroll, scroll);
    assert_eq!(fixture.animation.target(), target);
    assert!(fixture.reader.operator.is_none());
    assert_eq!(fixture.reader.message, "");
    assert_eq!(fixture.reader.desired, Some(9));
    assert_eq!(fixture.target, None);
  }
}

#[test]
fn wheel_keeps_visual_selection_stationary() -> Result<()> {
  for keyboard in [false, true] {
    let mut fixture = Fixture::new();
    fixture.reader.visual(SelectionKind::Character);
    let selected = fixture.reader.selection;
    fixture.reader.scroll = 50.0;
    fixture.animation.start(200.0, Instant::now());
    let input = if keyboard {
      ScrollInput::Keyboard { follow: true }
    } else {
      ScrollInput::Wheel
    };
    fixture.view().scroll(40.0, input)?;
    assert_eq!(fixture.reader.scroll, 50.0);
    assert_eq!(fixture.animation.target(), Some(240.0));
    assert_eq!(fixture.reader.selection, selected);
    if keyboard {
      assert!(fixture.source.reads > 0);
      assert_ne!(fixture.reader.cursor, Position::default());
      assert_eq!(fixture.target, Some(fixture.reader.cursor));
    } else {
      assert_eq!(fixture.source.reads, 0);
      assert_eq!(fixture.reader.cursor, Position::default());
      assert_eq!(fixture.target, None);
    }
  }
  Ok(())
}

#[test]
fn text_uses_full_pane_height() -> Result<()> {
  let mut fixture = Fixture::new();
  fixture.frame.visible.rows = 2;
  let position = Position { line: 4, ..Position::default() };
  assert!(!fixture.view().visible(position)?);
  fixture.frame.pane.text = true;
  assert!(fixture.view().visible(position)?);
  fixture.view().scroll(16.0, ScrollInput::Wheel)?;
  assert_eq!(fixture.text.top().line, 2);
  assert!(!fixture.animation.active());
  Ok(())
}

#[test]
fn reveal_cancels_before_source_failure() {
  for text in [false, true] {
    for operation in ["reveal", "land", "align"] {
      let mut fixture = Fixture::new();
      fixture.frame.pane.text = text;
      fixture.source.fail = 1;
      fixture.animation.start(200.0, Instant::now());
      let result = match operation {
        "reveal" => fixture.view().reveal(),
        "land" => fixture.view().land(),
        _ => fixture.view().align(0.5),
      };
      assert!(result.is_err(), "{operation}");
      assert_eq!(fixture.source.reads, 1);
      assert_eq!(fixture.animation.active(), text && operation != "reveal");
    }
  }
}
