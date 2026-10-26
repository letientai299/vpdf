use std::collections::BTreeMap;
use unicode_width::UnicodeWidthStr;
use vpdf_model::scene::Op;
use vpdf_model::style::Role;

pub fn text(scene: &[Op<'_>]) -> String {
  scene
    .iter()
    .filter_map(|op| match op {
      Op::Text(text) => Some(text.as_str().to_owned()),
      Op::Character(ch) => Some(ch.value().to_string()),
      Op::Spaces(count) => Some(" ".repeat(*count)),
      _ => None,
    })
    .collect()
}

pub fn spans(scene: &[Op<'_>]) -> Vec<(Role, String)> {
  let mut role = Role::Reset;
  let mut spans = Vec::new();
  for op in scene {
    if let Op::Style(next) = op {
      role = *next;
    }
    let text = text(std::slice::from_ref(op));
    if !text.is_empty() {
      spans.push((role, text));
    }
  }
  spans
}

/// Painted text by row; asserts it stays within `width`.
pub fn lines(scene: &[Op<'_>], width: u16) -> BTreeMap<usize, String> {
  let mut lines = BTreeMap::<usize, String>::new();
  let (mut column, mut row) = (0, 0);
  for op in scene {
    if let Op::Move { column: x, row: y } = op {
      (column, row) = (*x, *y);
      continue;
    }
    let text = text(std::slice::from_ref(op));
    if text.is_empty() {
      continue;
    }
    assert!(column + text.width() <= usize::from(width), "row {row}: {text:?}");
    if column == 0 {
      lines.insert(row, String::new());
    }
    lines.entry(row).or_default().push_str(&text);
    column += text.width();
  }
  lines
}

pub fn empty() -> vpdf_model::scene::Recorder<
  'static,
  impl FnMut(vpdf_model::tile::TileId) -> bool,
> {
  vpdf_model::scene::Recorder::new(|_| false)
}

pub(crate) use crate::fixtures;
pub(crate) use crate::fixtures::screen;

#[path = "../../../tests/support/page.rs"]
pub(crate) mod page;

pub fn text_fixture(rows: &[&str]) -> std::sync::Arc<vpdf_model::text::Text> {
  fixtures::lines(rows.iter().map(|row| fixtures::plain(row)))
}
