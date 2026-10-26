use super::layout::{Layout, Meta};
use super::*;
use crate::testing::lines;
use vpdf_model::geometry::At;
use vpdf_model::scene::Op;
use vpdf_model::style::Role;

fn figure(depth: usize) -> Row {
  Row {
    key: "Fig 1".into(),
    kind: RowKind::Figure,
    depth,
    ..Row::new(
      "The Transformer – model architecture".into(),
      "p123".into(),
      Destination::Position(Position::default()),
    )
  }
}

#[test]
fn narrow_rows_drop_meta_in_order() {
  // (available, key, glyph, page, badge, title room)
  let cases = [
    (20, "Fig 1 ", true, " p123", " +4", 4),
    (19, "Fig 1 ", true, " p123", "", 6),
    (14, "Fig 1 ", true, " p123", "", 1),
    (13, "Fig 1 ", true, "", "", 5),
    (12, "Fig 1 ", true, "", "", 4),
    (11, "Fig 1 ", false, "", "", 5),
    (10, "Fig 1 ", false, "", "", 4),
    (9, "", false, "", "", 9),
  ];
  for (available, key, glyph, page, badge, room) in cases {
    let meta = Meta::new(&figure(0), available, 5, 0, 4);
    let got = (
      meta.key.as_str(),
      meta.glyph.is_some(),
      meta.page.as_str(),
      meta.badge.as_str(),
      meta.room,
    );
    assert_eq!(got, (key, glyph, page, badge, room), "{available}");
  }
}

#[test]
fn meta_edge_cases() {
  let plain = |key: &str, detail: &str| Row {
    key: key.into(),
    ..Row::new("label".into(), detail.into(), Destination::None)
  };
  // A keyless row pads the key column.
  let meta = Meta::new(&plain("", "p1"), 20, 5, 0, 0);
  assert_eq!(
    (meta.key.as_str(), meta.page.as_str(), meta.room),
    ("      ", " p1", 11)
  );
  // `+N` outlives a page too wide to show.
  let meta = Meta::new(&plain("", &"x".repeat(20)), 20, 0, 0, 4);
  assert_eq!(
    (meta.page.as_str(), meta.badge.as_str(), meta.room),
    ("", " +4", 17)
  );
  // Indent leaves the title its share.
  let meta = Meta::new(&plain("", ""), 10, 0, 5, 0);
  assert_eq!((meta.indent, meta.room), (6, 4));
}

#[test]
fn jump_lines_keep_glyph_and_key() {
  let row = Row { detail: "p. 5".into(), ..figure(0) };
  let layout = Layout::jumps(&[row], 28, 0);
  let first = &layout.lines[0].row;
  assert_eq!((first.key.as_str(), first.kind), ("Fig 1", RowKind::Figure));
  assert_eq!(first.detail, "");
  assert!(layout.lines[1..].iter().all(|line| line.row.key.is_empty()));
}

#[test]
fn rows_keep_the_page_apart() {
  let flat = |width| {
    let mut panel = Panel::new(PanelKind::Figures, vec![figure(0)]);
    let mut scene = Vec::new();
    panel.scene_list(&mut scene, At::default(), width, 4);
    lines(&scene, width)[&1].clone()
  };
  let grouped = |width| {
    let heading = Row { depth: 0, ..Row::parse("Page 3") };
    let mut panel = Panel::new(PanelKind::Figures, vec![heading, figure(1)]);
    panel.group(&[]);
    let mut scene = Vec::new();
    panel.scene_list(&mut scene, At::default(), width, 4);
    let lines = lines(&scene, width);
    assert!(lines[&1].starts_with(" ▾ Page 3"), "{width}: {:?}", lines[&1]);
    lines[&2].clone()
  };
  // (width, flat row, grouped row)
  let cases = [
    (28, " ▣ Fig 1 The Transform p123", "     ▣ Fig 1 The Tran… p123"),
    (
      39,
      " ▣ Fig 1 The Transformer – model  p123",
      "     ▣ Fig 1 The Transformer – m… p123",
    ),
    (
      50,
      " ▣ Fig 1 The Transformer – model architectur p123",
      "     ▣ Fig 1 The Transformer – model archit… p123",
    ),
  ];
  for (width, want_flat, want_grouped) in cases {
    assert_eq!(flat(width), want_flat, "{width}");
    assert_eq!(grouped(width), want_grouped, "{width}");
  }
}

#[test]
fn jump_positions_skip_the_key_column() {
  let position = "p. 5 · L1 C1 abcdefghijk";
  let row = Row {
    key: "−2".into(),
    ..Row::new("Heading: Intro".into(), position.into(), Destination::None)
  };
  let mut panel = Panel::new(PanelKind::Jumps, vec![row]);
  let mut scene = Vec::new();
  panel.scene_list(&mut scene, At::default(), 28, 6);
  let lines = lines(&scene, 28);
  assert!(lines[&1].starts_with(" −2 Heading: Intro"), "{:?}", lines[&1]);
  // Under the title, past the marker and key column.
  assert_eq!(lines[&2].trim_end(), "    p. 5 · L1 C1");
  assert_eq!(lines[&3].trim_end(), "    abcdefghijk");
}

#[test]
fn empty_panels_say_why() {
  // (kind, loading, topic, line)
  let cases = [
    (PanelKind::Outline, true, "", "Preparing contents…"),
    (PanelKind::Outline, false, "", "No headings. Use page jumps or search."),
    (PanelKind::Figures, true, "", "Finding figures…"),
    (PanelKind::Figures, false, "", "No captioned figures or tables."),
    (PanelKind::Results, true, "attention", "Searching…"),
    (PanelKind::Results, false, "attention", "No matches for “attention”."),
    (PanelKind::Results, false, "", "No matches."),
    (PanelKind::Annotations, true, "", "Reading annotations…"),
    (PanelKind::Annotations, false, "", "No annotations in this PDF."),
    (PanelKind::Jumps, false, "", "No jumps yet."),
    (PanelKind::Marks, false, "", "No marks. Press m and a letter."),
    (PanelKind::Info, false, "", ""),
  ];
  for (kind, loading, topic, want) in cases {
    let mut panel = Panel::new(kind, Vec::new());
    panel.set_note(if loading { "loading" } else { "" });
    panel.set_topic(topic);
    let mut scene = Vec::new();
    panel.scene_list(&mut scene, At::default(), 50, 4);
    let shown = lines(&scene, 50);
    let line =
      shown.get(&1).map_or("", |line| line.trim_end_matches(['│', ' ']));
    assert_eq!(line, want, "{kind:?} {loading}");
    let mut scene = Vec::new();
    panel.scene_list(&mut scene, At::default(), 50, 1);
    assert!(!lines(&scene, 50).contains_key(&1), "{kind:?} short");
  }
}

#[test]
fn current_row_maps_to_its_shown_row() {
  let depths = [0, 1, 1, 0, 1];
  let rows: Vec<_> =
    depths.into_iter().map(|depth| Row { depth, ..figure(depth) }).collect();
  // (grouped, closed heading, current, marked line)
  let cases = [
    (false, None, Some(2), Some(3)),
    (false, None, None, None),
    (false, None, Some(9), None),
    (true, None, Some(2), Some(3)),
    (true, Some(0), Some(2), Some(1)),
    (true, Some(0), Some(4), Some(3)),
    (true, Some(3), Some(4), Some(4)),
  ];
  for (grouped, closed, current, want) in cases {
    let mut panel = Panel::new(PanelKind::Figures, rows.clone());
    if grouped {
      panel.group(&[]);
    }
    if let Some(source) = closed {
      panel.choose(source);
      assert!(panel.fold(crate::keys::Fold::Close, 1));
    }
    panel.choose(1);
    panel.mark_current(current);
    let mut scene = Vec::new();
    panel.scene_list(&mut scene, At::default(), 40, 8);
    let marked: Vec<_> = lines(&scene, 40)
      .into_iter()
      .filter(|(_, line)| line.starts_with('▌'))
      .map(|(line, _)| line)
      .collect();
    let case = format!("{grouped} {closed:?} {current:?}");
    assert_eq!(marked, Vec::from_iter(want), "{case}");
    assert_eq!(panel.selected(), 1, "{case}");
  }
}

#[test]
fn refills_keep_the_current_row() {
  let row = |label: &str| {
    Row::new(
      label.into(),
      String::new(),
      Destination::Position(Position::default()),
    )
  };
  // (new rows, current after)
  let cases: [(&[&str], Option<usize>); 3] = [
    (&["a", "b", "c"], Some(1)),
    (&["z", "a", "b"], Some(2)),
    (&["a", "c"], None),
  ];
  for (labels, want) in cases {
    let mut panel =
      Panel::new(PanelKind::Figures, ["a", "b", "c"].map(row).to_vec());
    panel.mark_current(Some(1));
    panel.refill(labels.iter().map(|label| row(label)).collect(), "");
    assert_eq!(panel.current, want, "{labels:?}");
  }
  let mut panel = Panel::new(PanelKind::Outline, vec![row("a")]);
  panel.mark_current(Some(0));
  panel.refill(vec![row("a")], "");
  assert_eq!(panel.current, None);
  let mut panel = Panel::new(PanelKind::Jumps, vec![row("a")]);
  for width in 0..4 {
    panel.scene_list(&mut Vec::new(), At::default(), width, 4);
  }
}

fn keyed(key: &str, kind: RowKind, label: &str) -> Row {
  Row {
    key: key.into(),
    kind,
    ..Row::new(label.into(), "p2".into(), Destination::None)
  }
}

#[test]
fn row_search_finds_and_marks_keys() {
  let rows = vec![
    keyed("Fig 1", RowKind::Figure, "Encoder"),
    keyed("Tab 3", RowKind::Table, "Costs"),
    keyed("Fig 2", RowKind::Figure, "Decoder"),
  ];
  // (query, selected)
  for (query, selected) in
    [("tab 3", 1), ("table 3", 1), ("figure 2", 2), ("fig 2 dec", 2)]
  {
    let mut panel = Panel::new(PanelKind::Figures, rows.clone());
    panel.begin_search(false);
    assert!(panel.finish_search(query), "{query}");
    assert_eq!(panel.selected(), selected, "{query}");
  }
  let mut panel = Panel::new(PanelKind::Figures, rows);
  panel.begin_search(false);
  assert!(panel.finish_search("tab 3"));
  let mut scene = Vec::new();
  panel.scene_list(&mut scene, At::default(), 40, 6);
  let marked = scene.windows(2).any(|ops| {
    matches!(ops, [Op::Style(Role::TextMatch), Op::Text(text)]
      if text.as_str() == "Tab 3")
  });
  assert!(marked);
}

/// A projected keyed list paints the cut its layout reports.
#[test]
fn projected_keyed_rows_report_their_cut() {
  let label = "The Transformer – model architecture";
  let rows = vec![
    keyed("Fig 1", RowKind::Figure, label),
    keyed("Fig 2", RowKind::Figure, label),
    keyed("Tab 3", RowKind::Table, "Short"),
  ];
  let tree = tree::Tree::from_depths([0, 1, 0].into_iter());
  for width in 16..48 {
    let mut panel = Panel::new(PanelKind::Figures, rows.clone());
    panel.project(tree.project(|source| source == 0));
    for (line, source, title) in [(1, 0, label), (2, 2, "Short")] {
      panel.point(Pointing::Row(source));
      let mut scene = Vec::new();
      panel.scene_list(&mut scene, At::default(), width, 6);
      let shown = &lines(&scene, width)[&line];
      assert_eq!(
        panel.hover_title().is_some(),
        !shown.contains(title),
        "{width} {source}: {shown:?}"
      );
    }
  }
}

#[test]
fn ascii_glyphs_stay_distinct() {
  use RowKind as K;
  let kinds = [
    K::Figure,
    K::Table,
    K::Mention,
    K::Section,
    K::Search,
    K::Link,
    K::Note,
    K::Underline,
    K::Highlight,
  ];
  let glyphs: Vec<_> =
    kinds.iter().map(|kind| kind.glyph_in(true).unwrap()).collect();
  for ((glyph, role), kind) in glyphs.iter().zip(kinds) {
    assert!(glyph.is_ascii_graphic(), "{kind:?}");
    assert_eq!(Some(*role), kind.glyph_in(false).map(|(_, role)| role));
  }
  let unique: std::collections::BTreeSet<_> =
    glyphs.iter().map(|(glyph, _)| glyph).collect();
  assert_eq!(unique.len(), kinds.len());
  assert_eq!(K::Plain.glyph_in(true), None);
}

#[test]
fn copies_keep_caption_keys() {
  let mark = Row { key: "’a".into(), ..figure(0) };
  let mark = Row { kind: RowKind::Plain, ..mark };
  for (row, copied) in [(figure(0), "Fig 1 "), (mark, "")] {
    let label = row.label.trim().to_owned();
    let panel = Panel::new(PanelKind::Figures, vec![row]);
    assert_eq!(panel.yank_text(), Some(format!("{copied}{label}\n")));
  }
}
