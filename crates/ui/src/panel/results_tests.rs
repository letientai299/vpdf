use super::*;
use crate::testing::lines;
use unicode_width::UnicodeWidthStr;
use vpdf_model::geometry::At;
use vpdf_model::scene::Op;
use vpdf_model::style::Role;
use vpdf_search::search::Hit;

fn hit(page: usize, line: usize, excerpt: &str, needle: &str) -> Hit {
  let start = excerpt.find(needle).expect("needle");
  let first = Position { page: crate::text::PageId(page), line, glyph: 0 };
  Hit {
    first,
    last: first,
    excerpt: excerpt.into(),
    matched: start..start + needle.len(),
  }
}

fn results(topic: &str) -> Panel {
  let hits = [
    hit(0, 0, "Recurrent models use attention here", "attention"),
    hit(0, 1, "attention is all", "attention"),
    hit(2, 0, "self-attention layers", "attention"),
  ];
  let rows = Row::results(&hits, |page| (page.0 + 1).to_string());
  let mut panel = Panel::new(PanelKind::Results, rows);
  panel.set_topic(topic);
  panel.group(&[]);
  panel
}

#[test]
fn excerpts_clip_around_the_match() {
  let label = "Recurrent models use attention mechanisms here";
  let start = label.find("attention").unwrap();
  // (room, shown, match)
  let cases = [
    (60, label, "attention"),
    (30, "…dels use attention mechanism…", "attention"),
    (12, "…attention …", "attention"),
    (9, "attention", "attention"),
    (5, "atte…", "atte…"),
  ];
  for (room, shown, matched) in cases {
    let (text, range) = rows::excerpt(label, start..start + 9, room);
    assert_eq!(text, shown, "{room}");
    assert_eq!(&text[range], matched, "{room}");
  }
  let (text, range) = rows::excerpt("abc", 1..9, 10);
  assert_eq!((text.as_str(), range), ("abc", 0..0));
  // Emoji sequences measure wider than their characters.
  let label = "✔\u{FE0F} ok ✔\u{FE0F} done hit";
  let start = label.find("hit").unwrap();
  for room in 3..12 {
    let (text, range) = rows::excerpt(label, start..start + 3, room);
    assert!(text.width() <= room, "{room}: {text:?}");
    assert_eq!(&text[range], "hit", "{room}");
  }
}

#[test]
fn hits_group_by_page() {
  let panel = results("attention");
  let rows: Vec<_> =
    panel.rows().iter().map(|row| (row.depth, row.follows())).collect();
  assert_eq!(rows, [(0, false), (1, true), (1, true), (0, false), (1, true)]);
  assert_eq!(panel.rows()[3].label, "p3");
  assert_eq!(panel.row_count(), 3);
}

#[test]
fn long_topics_leave_room_for_the_note() {
  let mut panel = results("self-attention mechanism");
  panel.set_note("loading");
  let mut scene = Vec::new();
  panel.scene_list(&mut scene, At::default(), 32, 4);
  let header = &lines(&scene, 32)[&0];
  assert!(header.contains(" “s… loading"), "{header:?}");
}

#[test]
fn refills_from_empty_select_the_landed_hit() {
  let rows = results("attention").rows().to_vec();
  let landed = rows[4].destination.clone();
  let Destination::Position(landed) = landed else { unreachable!() };
  // (landed, selected)
  for (landed, selected) in [(None, 1), (Some(landed), 4)] {
    let mut panel = Panel::new(PanelKind::Results, Vec::new());
    panel.regroup(rows.clone(), "", "attention", landed);
    assert_eq!(panel.selected(), selected, "{landed:?}");
    assert_eq!(panel.current(), landed.map(|_| 4));
  }
}

#[test]
fn rows_paint_the_query_and_match() {
  let mut panel = results("attention");
  let mut scene = Vec::new();
  panel.scene_list(&mut scene, At::default(), 40, 8);
  let shown = lines(&scene, 40);
  assert!(shown[&0].starts_with(" RESULTS  3 “attention”"), "{:?}", shown[&0]);
  assert!(shown[&1].starts_with(" ▾ p1"), "{:?}", shown[&1]);
  assert!(
    shown[&2].contains("…current models use attention here"),
    "{:?}",
    shown[&2]
  );
  let matches = scene.windows(2).filter(|ops| {
    matches!(ops, [Op::Style(Role::TextMatch), Op::Text(text)]
      if text.as_str() == "attention")
  });
  assert_eq!(matches.count(), 3);
  let accent = scene.windows(2).any(|ops| {
    matches!(ops, [Op::Style(Role::Key), Op::Text(text)]
      if text.as_str() == " “attention”")
  });
  assert!(accent);
}

#[test]
fn refills_keep_folds_for_the_same_topic() {
  // (topic after the refill, p3 stays closed)
  for (topic, closed) in [("attention", true), ("model", false)] {
    let mut panel = results("attention");
    panel.choose(3);
    assert!(panel.fold(crate::keys::Fold::Close, 1));
    let rows = results(topic).rows().to_vec();
    panel.regroup(rows, "", topic, None);
    assert_eq!(panel.branch_closed(3), Some(closed), "{topic}");
    assert_eq!(panel.branch_closed(0), Some(false), "{topic}");
  }
}
