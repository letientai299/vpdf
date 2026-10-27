#[path = "../../../../tests/support/panel.rs"]
mod fixture;
use super::*;
use crate::text::PageId;
use fixture::panel;
use vpdf_model::style::Role;

#[test]
fn recorded_panels_outlive_their_source() {
  let at = vpdf_model::geometry::At { column: 7, row: 3 };
  let scene = {
    let mut source = panel();
    let mut scene = vpdf_model::scene::Scene::new();
    source.scene_list(&mut scene, at, 20, 6);
    assert_eq!(source.row_at(0, 0), None);
    assert_eq!(source.row_at(0, 1), Some(0));
    assert_eq!(source.row_at(0, 2), Some(1));
    assert_eq!(source.row_at(19, 2), None);
    scene
  };
  assert!(crate::testing::text(&scene).contains("日本語"));
}

#[test]
fn hover_requires_destination() {
  for kind in [
    PanelKind::Info,
    PanelKind::Version,
    PanelKind::Outline,
    PanelKind::Results,
    PanelKind::Recent,
    PanelKind::Marks,
    PanelKind::Jumps,
    PanelKind::Annotations,
  ] {
    for focused in [true, false] {
      for follows in [true, false] {
        let mut row = Row::parse("Search: a long label clipped by the panel");
        if follows {
          row.destination = Destination::Link(Target::Page(PageId(0)));
        }
        let mut panel = Panel::new(kind, vec![Row::parse("# Reader"), row]);
        panel.focus(focused);
        panel.point(Pointing::Row(1));
        let mut scene = vpdf_model::scene::Scene::new();
        panel.scene_list(
          &mut scene,
          vpdf_model::geometry::At::default(),
          20,
          6,
        );
        let spans = crate::testing::spans(&scene);
        let hover = if focused { Role::Hover } else { Role::HoverDim };
        assert_eq!(
          spans.iter().any(|(role, _)| *role == hover),
          follows,
          "{kind:?}"
        );
        if kind == PanelKind::Info {
          assert!(panel.hover_title().is_some());
        }
      }
    }
  }
}

#[test]
fn hover_skips_indentation() {
  for style in [Role::Hover, Role::HoverDim] {
    let row = Row::parse("  Chapter");
    let mut out = Vec::new();
    assert_eq!(row.scene(&mut out, Some(style), 40, 0, ""), 9);
    let spans = crate::testing::spans(&out);
    let text = crate::testing::text(&out);
    assert_eq!(text, "  Chapter");
    assert!(spans.contains(&(style, "Chapter".into())));
  }
}

#[test]
fn screen_scroll_keeps_panel_context() {
  for kind in [PanelKind::Version, PanelKind::Results] {
    let mut panel =
      Panel::new(kind, (0..40).map(|_| Row::parse("j: down")).collect());
    panel.scene_list(
      &mut Vec::new(),
      vpdf_model::geometry::At::default(),
      40,
      10,
    );
    assert!(panel.navigate(Input::plain(Action::Screen(1)), 10));
    assert_eq!(panel.selected, 6);
    if kind == PanelKind::Version {
      assert_eq!(panel.top, 6);
    }
    assert!(panel.navigate(Input::plain(Action::Screen(-1)), 10));
    assert_eq!(panel.selected, 0);
    let counted = Input { count: 2, ..Input::plain(Action::Screen(1)) };
    assert!(panel.navigate(counted, 10));
    assert_eq!(panel.selected, 12);
  }
}

#[test]
fn counted_list_boundaries() {
  let mut panel =
    Panel::new(PanelKind::Results, (0..5).map(|_| Row::parse("row")).collect());
  for motion in [Motion::First, Motion::Last] {
    for (count, expected) in [(1, 0), (3, 2), (99, 4)] {
      let input =
        Input { count, explicit: true, ..Input::plain(Action::Motion(motion)) };
      assert!(panel.navigate(input, 10));
      assert_eq!(panel.selected, expected);
    }
  }
  panel.navigate(Input::plain(Action::Motion(Motion::First)), 10);
  assert_eq!(panel.selected, 0);
  panel.navigate(Input::plain(Action::Motion(Motion::Last)), 10);
  assert_eq!(panel.selected, 4);
}

/// Equal labels part by destination; wrapped rows keep their line;
/// a panel search's origin follows its row.
#[test]
fn refills_follow_rows() {
  let at =
    |line| Destination::Position(Position { line, ..Position::default() });
  let row = |label: &str, line| Row::new(label.into(), String::new(), at(line));
  let twins = || vec![row("same", 1), row("same", 2)];
  let mut panel = Panel::new(PanelKind::Results, twins());
  (panel.selected, panel.clicked) = (1, Some(0));
  panel.begin_search(false);
  let mut more = twins();
  more.insert(0, row("new", 0));
  assert!(panel.refill(more, ""));
  assert_eq!((panel.selected, panel.clicked), (2, Some(1)));
  panel.abort_search();
  assert_eq!(panel.selected, 2, "the origin moved too");
  // A clicked row that leaves ends its double click.
  panel.refill(vec![row("same", 2)], "");
  assert_eq!(panel.clicked, None);
  // Jump rows take two lines; `top` counts lines.
  let jump = |n: usize| Row::new(format!("jump {n}"), "p. 1".into(), at(n));
  let rows = |first| (first..9).map(jump).collect::<Vec<_>>();
  let mut panel = Panel::new(PanelKind::Jumps, rows(1));
  (panel.selected, panel.height) = (6, 12);
  panel.reflow(40);
  panel.top = panel.line_start(4);
  let line = panel.screen_line(6);
  assert!(panel.refill(rows(0), ""));
  panel.reflow(40);
  assert_eq!((panel.selected, panel.screen_line(7)), (7, line));
}

#[test]
fn refills_keep_the_selected_row() {
  type Case = (&'static [&'static str], (usize, usize, bool));
  let rows = |labels: &[&str]| -> Vec<Row> {
    labels.iter().map(|label| Row::parse(label)).collect()
  };
  // (new rows) -> (selected, top, kept)
  let cases: [Case; 6] = [
    (&["a", "b", "c", "d"], (2, 1, true)),
    (&["x", "a", "b", "c", "d"], (3, 2, true)),
    (&["b", "c", "d"], (1, 0, true)),
    (&["c", "a", "b", "c"], (3, 2, true)),
    (&["a", "b", "d"], (2, 1, false)),
    (&["a"], (0, 0, false)),
  ];
  for (labels, want) in cases {
    let mut panel = Panel::new(PanelKind::Results, rows(&["a", "b", "c", "d"]));
    (panel.selected, panel.top) = (2, 1);
    let kept = panel.refill(rows(labels), "");
    assert_eq!((panel.selected, panel.top, kept), want, "{labels:?}");
  }
}

#[test]
fn paths_keep_the_file_name() {
  let row = Row {
    style: Style::Path,
    ..Row::new("~/books/ml/deep.pdf".into(), String::new(), Destination::None)
  };
  // (room, visible text)
  for (room, want) in [
    (40, "~/books/ml/deep.pdf"),
    (14, "…s/ml/deep.pdf"),
    (9, "…deep.pdf"),
    (8, "deep.pdf"),
    (5, "deep."),
  ] {
    let mut out = Vec::new();
    let width = row.scene(&mut out, Some(Role::Reset), room, 0, "");
    let spans = crate::testing::spans(&out);
    let out = crate::testing::text(&out);
    let text = out;
    assert_eq!(text, want, "{room}");
    assert_eq!(width, want.width(), "{room}");
    assert!(
      spans
        .iter()
        .any(|(role, text)| *role == Role::File && text.starts_with("deep")),
      "{room}"
    );
  }
}
#[test]
fn presses_beyond_the_list_miss_its_rows() {
  let rows = ["a", "b", "c"].map(Row::parse).into();
  let mut panel = Panel::new(PanelKind::Recent, rows);
  panel.scene_list(
    &mut Vec::new(),
    vpdf_model::geometry::At::default(),
    20,
    10,
  );
  let press = |column| {
    MouseEvent::new(MouseEventKind::Down(MouseButton::Left), column, 2)
  };
  assert_eq!(panel.pointer(press(25)), None);
  assert_eq!(panel.pointer(press(5)), Some(1));
}

#[test]
fn panels_reserve_page_space() {
  let screen =
    Screen { origin: (3, 5), ..crate::testing::screen::cells(100, 24) };
  let outline = Panel::new(PanelKind::Outline, Vec::new());
  let help = Panel::new(PanelKind::Version, Vec::new());
  let recent = Panel::recent(Vec::new(), &[]);
  for (panel, dragged, left, columns) in [
    (None, None, 0, 100),
    (Some(&outline), None, 33, 67),
    (Some(&help), None, 50, 50),
    (Some(&outline), Some(1), 8, 92),
    (Some(&outline), Some(100), 88, 12),
    (Some(&recent), Some(100), 88, 12),
  ] {
    let (offset, page) = page_space(screen, panel, dragged);
    assert_eq!((offset, page.columns), (left, columns));
    assert_eq!(
      (page.rows, page.cell_width, page.cell_height, page.origin),
      (24, 8, 16, (3, 5))
    );
  }
  let mut recent = recent;
  recent.focus(false);
  let (left, page) = page_space(screen, Some(&recent), None);
  assert_eq!((left, page.columns), (33, 67));
}

#[test]
fn version_has_no_current_line() {
  for focused in [true, false] {
    let mut panel = Panel::new(
      PanelKind::Version,
      ["Reader description", "vpdf build"].map(Row::parse).into(),
    );
    panel.focus(focused);
    for selected in 0..2 {
      panel.selected = selected;
      let mut scene = vpdf_model::scene::Scene::new();
      panel.scene_list(&mut scene, vpdf_model::geometry::At::default(), 40, 10);
      assert!(
        !crate::testing::spans(&scene)
          .iter()
          .any(|(role, _)| { matches!(role, Role::Selected | Role::Current) })
      );
    }
  }
}

#[test]
fn version_sections_follow_urls() {
  let url = "https://example.com/commit/abc";
  let rows = Row::sections(&format!("Reader\n# Build\n{url}"));
  assert!(rows[2].destination == Destination::Link(Target::Uri(url.into())));
  assert_eq!(rows[2].depth, 1);
  assert!(
    Row::parse("See https://example.com").destination == Destination::None
  );
}

#[test]
fn sections_group_under_headings_with_default_folds() {
  let text = [
    "paper.pdf",
    "/tmp/docs",
    "2 pages · 1.0 MiB",
    "# Document",
    "Pages: 2",
    "# Render",
    "Cell: 8x16 px",
    "# Terminal",
    "Mouse: on",
  ]
  .join("\n");
  let rows = Row::sections(&text);
  let shape: Vec<_> = rows.iter().map(|row| (row.depth, row.style)).collect();
  assert_eq!(
    shape,
    [
      (0, Style::Plain),
      (0, Style::Muted),
      (0, Style::Muted),
      (0, Style::Heading),
      (1, Style::Key),
      (0, Style::Heading),
      (1, Style::Key),
      (0, Style::Heading),
      (1, Style::Key),
    ]
  );
  let mut panel = Panel::new(PanelKind::Info, Vec::new());
  panel.regroup(rows, "", "", None);
  // Render and Terminal start folded; the first value is selected.
  let closed: Vec<_> = (0..panel.rows().len())
    .filter(|&source| panel.branch_closed(source) == Some(true))
    .collect();
  assert_eq!(closed, [5, 7]);
  assert_eq!(panel.selected(), 4);
  assert_eq!(Panel::width(PanelKind::Info, 200, None), 50);
  assert_eq!(Panel::width(PanelKind::Version, 200, None), 64);
}

#[test]
fn folded_groups_start_with_a_shown_item() {
  let text = "vpdf 1\nA reader\n# Build\nsource abc\nhelp def";
  let panel = Panel::grouped(PanelKind::Version, Row::sections(text), 20);
  assert_eq!(panel.branch_closed(2), Some(true));
  // Nothing under Build shows, so the heading holds the selection.
  assert_eq!(panel.selected(), 2);
  assert_eq!(panel.row_roles(2).0, Role::Selected);
}
