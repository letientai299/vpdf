use super::*;
use crate::testing::fixtures::Source;
use crate::testing::text_fixture as text;
use crate::text::Bounds;
use vpdf_reader::reader::Reader;

fn at(page: usize, line: usize, glyph: usize) -> Position {
  Position { page: PageId(page), line, glyph }
}

fn view(
  pages: &mut Vec<Arc<Text>>,
  (height, columns): (usize, usize),
  top: Position,
) -> View {
  View::new(pages, (height, columns), top).expect("view")
}

/// The origin a `(height, columns)` view at `top` keeps for `cursor`.
fn kept(
  pages: &[Arc<Text>],
  top: Position,
  cursor: Position,
  size: (usize, usize),
) -> Result<Position> {
  let mut pages = pages.to_vec();
  view(&mut pages, size, top).keep_visible(cursor, &mut pages)
}

#[test]
fn view_moves_only_to_show_the_cursor() -> Result<()> {
  let mut rows = vec!["abcdefghij"; 28];
  rows.extend(["日本語日本語", ""]);
  let one = [text(&rows)];
  for (name, top, cursor, size, expected) in [
    ("inside", at(0, 5, 2), at(0, 9, 5), (10, 4), at(0, 5, 2)),
    ("first row", at(0, 5, 0), at(0, 5, 0), (10, 4), at(0, 5, 0)),
    ("last row", at(0, 5, 0), at(0, 14, 0), (10, 4), at(0, 5, 0)),
    ("above", at(0, 5, 0), at(0, 3, 0), (10, 4), at(0, 3, 0)),
    ("below", at(0, 5, 0), at(0, 15, 0), (10, 4), at(0, 6, 0)),
    ("past the end", at(0, 25, 0), at(0, 26, 0), (10, 4), at(0, 20, 0)),
    ("short page", at(0, 5, 0), at(0, 7, 0), (40, 4), at(0, 0, 0)),
    ("left", at(0, 0, 5), at(0, 0, 2), (10, 4), at(0, 0, 2)),
    ("last column", at(0, 0, 0), at(0, 0, 3), (10, 4), at(0, 0, 0)),
    ("right", at(0, 0, 0), at(0, 0, 7), (10, 4), at(0, 0, 4)),
    ("wide glyphs", at(0, 20, 0), at(0, 28, 4), (10, 4), at(0, 20, 6)),
    ("empty line", at(0, 20, 5), at(0, 29, 0), (10, 4), at(0, 20, 0)),
    ("no rows", at(0, 5, 2), at(0, 15, 7), (0, 4), at(0, 5, 2)),
    ("no columns", at(0, 5, 2), at(0, 15, 7), (10, 0), at(0, 5, 2)),
  ] {
    assert_eq!(kept(&one, top, cursor, size)?, expected, "{name}");
  }
  Ok(())
}

#[test]
fn view_follows_the_cursor_across_pages() -> Result<()> {
  let six = ["abcdefghij"; 6];
  let three = [text(&six), text(&six), text(&six[..3])];
  for (name, top, cursor, expected) in [
    ("next page", at(0, 0, 0), at(1, 2, 0), at(0, 6, 0)),
    ("page above", at(1, 3, 0), at(0, 4, 0), at(0, 4, 0)),
    ("over a break", at(0, 4, 0), at(1, 0, 0), at(0, 4, 0)),
    ("same page below", at(1, 0, 0), at(1, 5, 0), at(1, 2, 0)),
    ("pages away", at(0, 0, 0), at(2, 1, 0), at(1, 5, 0)),
  ] {
    assert_eq!(kept(&three, top, cursor, (4, 4))?, expected, "{name}");
  }
  Ok(())
}

#[test]
fn rows_step_over_breaks_and_stop_at_the_ends() -> Result<()> {
  // Lines, break, blank, break, lines.
  let mut pages = vec![text(&["a", "b", "c"]), text(&[]), text(&["d", "e"])];
  for (name, from, rows, expected) in [
    ("onto a break", at(0, 1, 0), 2, (at(0, 3, 0), 2)),
    ("onto an empty page", at(0, 1, 0), 3, (at(1, 0, 0), 3)),
    ("past an empty page", at(0, 1, 0), 5, (at(2, 0, 0), 5)),
    ("past the end", at(0, 1, 0), 10, (at(2, 1, 0), 6)),
    ("back over breaks", at(2, 0, 2), -3, (at(0, 3, 2), 3)),
    ("past the start", at(2, 1, 0), -10, (at(0, 0, 0), 7)),
    ("still", at(1, 0, 0), 0, (at(1, 0, 0), 0)),
  ] {
    assert_eq!(step(&mut pages, from, rows)?, expected, "{name}");
  }
  for (name, top, size, expected) in [
    ("fills from the end", at(2, 1, 0), (4, 4), at(1, 0, 0)),
    ("past a page's rows", at(0, 9, 0), (2, 4), at(0, 3, 0)),
    ("past the pages", at(5, 0, 0), (2, 4), at(2, 0, 0)),
    ("short document", at(1, 0, 0), (9, 4), at(0, 0, 0)),
    ("no rows", at(2, 1, 0), (0, 4), at(2, 1, 0)),
    ("narrow lines", at(0, 0, 3), (2, 4), at(0, 0, 0)),
  ] {
    assert_eq!(clamp(&mut pages, top, size)?, expected, "{name}");
  }
  Ok(())
}

#[test]
fn clicks_and_cells_match_drawn_glyphs() {
  let mut marks = vec![text(&["abcd\u{301}"])];
  let mut pages =
    vec![text(&["abc日", "abcdefghij", "日本語日本語", "", "xyz"])];
  let mut shown =
    |height, glyph| view(&mut pages, (height, 4), at(0, 0, glyph));
  for (name, left, (column, row), expected) in [
    ("first glyph", 0, (0, 0), at(0, 0, 0)),
    ("blank by a cut wide glyph", 0, (3, 0), at(0, 0, 2)),
    ("past the last column", 0, (9, 1), at(0, 1, 3)),
    ("wide glyph's second cell", 0, (1, 2), at(0, 2, 0)),
    ("next wide glyph", 0, (2, 2), at(0, 2, 1)),
    ("empty line", 0, (0, 3), at(0, 3, 0)),
    ("below the view", 0, (0, 6), at(0, 3, 0)),
    ("scrolled right", 2, (0, 1), at(0, 1, 2)),
    ("row left of the view", 5, (1, 0), at(0, 0, 3)),
  ] {
    let position = shown(3, left).position(column, row);
    assert_eq!(position, expected, "{name}");
  }
  for (name, left, position, expected) in [
    ("drawn", 0, at(0, 0, 2), Some((2, 0))),
    ("cut wide glyph", 0, at(0, 0, 3), None),
    ("after a wide glyph", 0, at(0, 2, 1), Some((2, 2))),
    ("clipped left glyph", 1, at(0, 2, 0), None),
    ("one cell pan", 1, at(0, 2, 1), Some((1, 2))),
    ("two cell pan", 2, at(0, 2, 1), Some((0, 2))),
    ("empty line", 0, at(0, 3, 0), None),
    ("below the view", 0, at(0, 4, 0), None),
    ("other page", 0, at(1, 0, 0), None),
    ("left of the view", 2, at(0, 1, 1), None),
    ("first after scrolling", 2, at(0, 1, 2), Some((0, 1))),
  ] {
    assert_eq!(shown(3, left).cell(position), expected, "{name}");
  }
  for (name, left, (column, row), expected) in [
    ("first glyph", 0, (0, 0), Some(at(0, 0, 0))),
    ("blank by a cut wide glyph", 0, (3, 0), None),
    ("past the last column", 0, (9, 1), None),
    ("wide glyph's second cell", 0, (1, 2), Some(at(0, 2, 0))),
    ("next wide glyph", 0, (2, 2), Some(at(0, 2, 1))),
    ("empty line", 0, (0, 3), None),
    ("last line", 0, (2, 4), Some(at(0, 4, 2))),
    ("below the view", 0, (0, 5), None),
    ("scrolled right", 2, (0, 1), Some(at(0, 1, 2))),
    ("row left of the view", 5, (1, 0), None),
  ] {
    assert_eq!(shown(5, left).hit(column, row), expected, "{name}");
  }
  assert_eq!(shown(2, 0).hit(0, 2), None, "below a short view");
  let mark = view(&mut marks, (3, 4), at(0, 0, 0));
  assert_eq!(mark.cell(at(0, 0, 4)), Some((3, 0)), "mark");
  assert_eq!(mark.hit(3, 0), Some(at(0, 0, 3)), "mark");
}

/// Rows past a page's lines show the next page after a break.
#[test]
fn rows_map_across_page_breaks() -> Result<()> {
  let mut pages = vec![text(&["ab", "cd"]), text(&["ef", "gh"])];
  let shown = view(&mut pages, (4, 4), at(0, 1, 0));
  for (name, cell, position, hit) in [
    ("line above the break", (1, 0), at(0, 1, 1), Some(at(0, 1, 1))),
    ("the break", (1, 1), at(0, 1, 1), None),
    ("next page", (1, 2), at(1, 0, 1), Some(at(1, 0, 1))),
    ("below the rows", (0, 5), at(1, 1, 0), None),
  ] {
    assert_eq!(shown.position(cell.0, cell.1), position, "{name}");
    assert_eq!(shown.hit(cell.0, cell.1), hit, "{name}");
  }
  for (name, position, cell, inside) in [
    ("first row", at(0, 1, 1), Some((1, 0)), false),
    ("after the break", at(1, 0, 0), Some((0, 2)), true),
    ("last row", at(1, 1, 1), Some((1, 3)), false),
    ("above the view", at(0, 0, 0), None, false),
  ] {
    assert_eq!(shown.cell(position), cell, "{name}");
    assert_eq!(shown.inside(position), inside, "{name}");
  }
  assert_eq!(shown.shown_pages(), 0..2);
  let mut empty = vec![text(&["ab"]), text(&[]), text(&["cd"])];
  let blank = view(&mut empty, (4, 4), at(0, 0, 0));
  assert_eq!(blank.position(1, 2), at(1, 0, 0), "blank row");
  assert_eq!(blank.position(1, 3), at(1, 0, 0), "empty page's break");
  assert_eq!(blank.hit(0, 2), None, "blank row");
  assert!(blank.inside(at(1, 0, 0)), "blank row");
  assert_eq!(blank.keep_visible(at(1, 0, 0), &mut empty)?, at(0, 0, 0));
  assert_eq!(view(&mut pages, (1, 4), at(0, 1, 0)).shown_pages(), 0..1);
  Ok(())
}

#[test]
fn aligning_puts_the_line_at_a_fraction() -> Result<()> {
  let one = vec![text(&["abcd"; 40])];
  let two = vec![text(&["abcd"; 5]), text(&["abcd"; 5])];
  for (pages, height, fraction, line, top) in [
    (&one, 10, 0.0, at(0, 30, 0), at(0, 30, 0)),
    (&one, 10, 0.5, at(0, 30, 0), at(0, 25, 0)),
    (&one, 10, 1.0, at(0, 30, 0), at(0, 21, 0)),
    (&one, 10, 1.0, at(0, 3, 0), at(0, 0, 0)),
    (&two, 4, 1.0, at(1, 1, 0), at(0, 4, 0)),
  ] {
    let mut pages = pages.clone();
    let shown = view(&mut pages, (height, 4), at(0, 0, 0));
    let top_at = shown.aligned(line, fraction, &mut pages)?;
    assert_eq!(top_at, top, "{fraction} {line:?}");
  }
  Ok(())
}

fn pages(text: Arc<Text>) -> Source {
  Source { text, count: 1, bounds: Bounds::default(), label: "ii" }
}

#[test]
fn recorded_advances_match_hits() -> Result<()> {
  let pages = pages(text(&["界e\u{301}x"]));
  let reader = Reader::default();
  let paint = Marks {
    reader: &reader,
    matches: &[],
    flash: None,
    landing: None,
    cursorline: false,
    dim: false,
    hovered: &[],
  };
  let frame = Frame {
    screen: crate::testing::screen::cells(4, 2),
    at: At { column: 7, row: 3 },
    left: 0,
  };
  let mut state = TextView::default();
  let mut pane = state.bind(pages, frame);
  let mut scene = vpdf_model::scene::Scene::new();
  pane.scene(&mut scene, &paint)?;
  let characters: Vec<_> = scene
    .iter()
    .filter_map(|op| match op {
      Op::Character(character) => {
        Some((character.value(), character.columns()))
      }
      _ => None,
    })
    .collect();
  assert_eq!(characters, [('界', 2), ('e', 1), ('\u{301}', 0), ('x', 1)]);
  assert_eq!(pane.shown(1, 0)?, Some(at(0, 0, 0)));
  assert_eq!(pane.shown(2, 0)?, Some(at(0, 0, 1)));
  assert_eq!(pane.shown(3, 0)?, Some(at(0, 0, 3)));
  assert_eq!(crate::testing::text(&scene), "界e\u{301}x");
  Ok(())
}

#[test]
fn hits_follow_pending_and_drawn_frames() -> Result<()> {
  let pages = pages(text(&["a", "b", "c", "d"]));
  let reader = Reader::default();
  let paint = Marks {
    reader: &reader,
    matches: &[],
    flash: None,
    landing: None,
    cursorline: false,
    dim: false,
    hovered: &[],
  };
  let frame = Frame {
    screen: crate::testing::screen::cells(6, 3),
    at: At::default(),
    left: 2,
  };
  let mut state = TextView::default();
  let mut pane = state.bind(pages, frame);
  pane.scene(&mut Vec::new(), &paint)?;
  pane.scroll(1)?;
  assert_eq!(pane.hit(2, 0)?, Some(at(0, 1, 0)));
  assert_eq!(pane.shown(2, 0)?, Some(at(0, 0, 0)));
  assert_eq!(pane.position(2, 0)?, at(0, 0, 0));
  assert_eq!(pane.cell(at(0, 1, 0))?, Some((2, 0)));
  pane.scene(&mut Vec::new(), &paint)?;
  assert_eq!(pane.shown(2, 0)?, Some(at(0, 1, 0)));
  pane.scroll(1)?;
  pane.state.invalidate();
  assert_eq!(pane.shown(2, 0)?, Some(at(0, 2, 0)));
  Ok(())
}

#[test]
fn cursorline_follows_reader_mode() -> Result<()> {
  for mode in [0, 1, 2] {
    let mut reader = Reader::default();
    match mode {
      1 => reader.enter_cursor(),
      2 => reader.visual(SelectionKind::Character),
      _ => {}
    }
    for enabled in [false, true] {
      let paint = Marks {
        reader: &reader,
        matches: &[],
        flash: None,
        landing: None,
        cursorline: enabled,
        dim: false,
        hovered: &[],
      };
      let pages = pages(text(&["abc", "other"]));
      let frame = Frame {
        screen: crate::testing::screen::cells(10, 2),
        at: At::default(),
        left: 0,
      };
      let mut state = TextView::default();
      let mut scene = Vec::new();
      state.bind(pages, frame).scene(&mut scene, &paint)?;
      let bands = scene
        .iter()
        .filter(|op| matches!(op, Op::Style(Role::TextCursorline)))
        .count();
      assert_eq!(bands > 0, enabled && mode != 0);
      assert!(!paint.on_cursorline(at(0, 1, 0)));
    }
  }
  Ok(())
}
