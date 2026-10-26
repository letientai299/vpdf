use super::*;
use crate::testing::lines;
use vpdf_model::geometry::At;
use vpdf_model::style::Role;

fn note(kind: &str, contents: &str, color: Option<[u8; 3]>) -> Annotation {
  Annotation {
    kind: kind.into(),
    areas: Vec::new(),
    contents: contents.into(),
    author: String::new(),
    color,
  }
}

fn row(label: &str, annotation: Annotation) -> Row {
  crate::annotations::entry(label.into(), Position::default(), annotation)
}

#[test]
fn kinds_pick_glyph_and_color() {
  let yellow = [255, 235, 59];
  // (kind, color, glyph, role)
  let cases = [
    ("Highlight", Some(yellow), '▇', Role::Swatch(yellow)),
    ("Highlight", None, '▇', Role::Key),
    ("Underline", None, '‗', Role::Argument),
    ("Squiggly", Some(yellow), '‗', Role::Swatch(yellow)),
    ("StrikeOut", None, '‗', Role::Argument),
    ("Note", None, '✎', Role::Key),
    ("FreeText", None, '✎', Role::Key),
  ];
  for (kind, color, glyph, role) in cases {
    let row = row("text", note(kind, "", color));
    assert_eq!(row.glyph(), Some((glyph, role)), "{kind}");
  }
}

#[test]
fn only_the_selected_row_shows_its_note() {
  let rows = vec![
    (
      crate::text::PageId(0),
      row("quoted", note("Highlight", "first note", None)),
    ),
    (
      crate::text::PageId(0),
      row("other", note("Highlight", "second note", None)),
    ),
    (
      crate::text::PageId(0),
      row("plain note", note("Note", "plain note", None)),
    ),
  ];
  let rows = Row::paged(rows, |page| (page.0 + 1).to_string());
  // (selected, lines below the heading)
  let cases: [(usize, &[&str]); 3] = [
    (1, &["quoted", "first note", "other", "plain note"]),
    (2, &["quoted", "other", "second note", "plain note"]),
    (3, &["quoted", "other", "plain note"]),
  ];
  for (selected, want) in cases {
    let mut panel = Panel::new(PanelKind::Annotations, rows.clone());
    panel.group(&[]);
    panel.choose(selected);
    let mut scene = Vec::new();
    panel.scene_list(&mut scene, At::default(), 40, 10);
    let shown = lines(&scene, 40);
    let got: Vec<_> = (2..2 + want.len())
      .map(|line| shown[&line].trim_matches([' ', '│', '▇', '✎']).to_owned())
      .collect();
    assert_eq!(got, want, "{selected}");
    if selected == 1 {
      // Gutter, chevron, indent and glyph come first.
      assert!(shown[&3].starts_with("       ✎ first note"), "{:?}", shown[&3]);
    }
    let below = shown
      .get(&(2 + want.len()))
      .map_or("", |line| line.trim_matches([' ', '│']));
    assert_eq!(below, "", "{selected}");
  }
}

#[test]
fn bare_annotations_name_their_kind() {
  let text = crate::text::Text::default();
  let page = crate::text::PageId(0);
  let ink = crate::annotations::row(&text, page, note("Ink", "", None));
  assert_eq!((ink.label.as_str(), ink.style), ("Ink", Style::Muted));
  let noted = crate::annotations::row(&text, page, note("Ink", "loop", None));
  assert_eq!((noted.label.as_str(), noted.style), ("loop", Style::Plain));
  // The painted line stays muted.
  let layout = Layout::annotations(&[ink], 40, 1);
  assert_eq!(layout.lines[0].row.style, Style::Muted);
}

fn noted_rows(pages: &[usize]) -> Vec<Row> {
  let rows = pages.iter().enumerate().map(|(index, &page)| {
    let label = format!("quote {index}");
    let contents = format!("note {index}");
    (crate::text::PageId(page), row(&label, note("Highlight", &contents, None)))
  });
  Row::paged(rows, |page| (page.0 + 1).to_string())
}

#[test]
fn note_lines_belong_to_their_row() {
  let mut panel = Panel::new(PanelKind::Annotations, noted_rows(&[0, 0, 1]));
  panel.group(&[]);
  panel.choose(1);
  panel.scene_list(&mut Vec::new(), At::default(), 40, 10);
  // Heading, row 1, its note, row 2.
  assert_eq!(panel.row_at(8, 3), Some(1));
  assert_eq!(panel.row_at(8, 4), Some(2));
  // A closed group above keeps the expanded row's note under it.
  panel.choose(4);
  assert!(panel.fold(crate::keys::Fold::CollapseAll, 1));
  assert!(panel.fold(crate::keys::Fold::Open, 1));
  panel.choose(4);
  let mut scene = Vec::new();
  panel.scene_list(&mut scene, At::default(), 40, 10);
  let shown = lines(&scene, 40);
  assert!(shown[&1].contains("p1"), "{:?}", shown[&1]);
  assert!(shown[&3].contains("quote 2"), "{:?}", shown[&3]);
  assert!(shown[&4].contains("note 2"), "{:?}", shown[&4]);
}

/// A click below the old selection keeps the clicked row in place as
/// the old note collapses.
#[test]
fn new_selections_keep_their_line() {
  let mut panel = Panel::new(PanelKind::Annotations, noted_rows(&[0; 10]));
  panel.group(&[]);
  panel.choose(5);
  panel.scene_list(&mut Vec::new(), At::default(), 40, 8);
  panel.top = 4;
  panel.scene_list(&mut Vec::new(), At::default(), 40, 8);
  let line = panel.screen_line(7);
  assert_eq!(line, 4);
  panel.choose(7);
  panel.scene_list(&mut Vec::new(), At::default(), 40, 8);
  assert_eq!(panel.screen_line(7), line);
}

/// Wrapped lines start under the title at any width.
#[test]
fn wrapped_lines_align_with_the_title() {
  let label = "alpha bravo charlie delta echo foxtrot golf hotel";
  let rows =
    [(crate::text::PageId(0), row(label, note("Highlight", "", None)))];
  let rows = Row::paged(rows, |page| (page.0 + 1).to_string());
  for width in 8..=40u16 {
    let mut panel = Panel::new(PanelKind::Annotations, rows.clone());
    panel.group(&[]);
    panel.choose(1);
    let mut scene = Vec::new();
    panel.scene_list(&mut scene, At::default(), width, 12);
    let shown = lines(&scene, width);
    let column =
      |line: &str| line.chars().position(|ch| ch.is_ascii_alphabetic());
    let Some((&first, title)) =
      shown.iter().find(|(_, line)| line.contains("al"))
    else {
      continue;
    };
    let title = column(title);
    assert_eq!(column(&shown[&(first + 1)]), title, "{width}: {shown:?}");
  }
}
