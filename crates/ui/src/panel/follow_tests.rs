use super::*;
use crate::find::Search;
use crate::reading::Pane;
#[cfg(test)]
use anyhow::Result;
use vpdf_model::input::KeyModifiers;
use vpdf_reader::reader::{Mode, SelectionKind};
use vpdf_search::{query::Query, search::Hit};

fn panel(kind: PanelKind, destination: Destination) -> Panel {
  Panel::new(
    kind,
    vec![Row::new("  Heading  ".into(), String::new(), destination)],
  )
}

fn annotation() -> Annotation {
  Annotation {
    kind: "Note".into(),
    author: "Reader".into(),
    contents: "A note".into(),
    areas: Vec::new(),
  }
}

#[test]
fn follow_preserves_descriptions() {
  let position = Position::default();
  let query = Query { text: "word".into(), ..Query::default() };
  let mut search = Search::default();
  search.show(
    query.clone(),
    vec![Hit { first: position, last: position, excerpt: "word".into() }],
  );
  let result = panel(PanelKind::Results, Destination::Position(position));
  for complete in [true, false] {
    if !complete {
      search.begin(query.clone(), 2, position);
    }
    assert_eq!(
      result.follow(&search),
      Some(Follow::Land {
        position,
        description: Some(search.description(position)),
      })
    );
  }
  let plain = panel(PanelKind::Marks, Destination::Position(position));
  assert_eq!(
    plain.follow(&search),
    Some(Follow::Land { position, description: None })
  );
  let description = Some(JumpDescription::Heading("saved".into()));
  let saved = panel(
    PanelKind::Jumps,
    Destination::Jump { position, description: description.clone() },
  );
  assert_eq!(
    saved.follow(&search),
    Some(Follow::Land { position, description })
  );
}

#[test]
fn headings_describe_internal_links() {
  let search = Search::default();
  let target = Target::Page(crate::text::PageId(0));
  let mut outline =
    panel(PanelKind::Outline, Destination::Link(target.clone()));
  assert_eq!(
    outline.follow(&search),
    Some(Follow::Link {
      target: target.clone(),
      description: Some(JumpDescription::Heading("Heading".into())),
    })
  );
  outline.rows[0].label = "  ".into();
  assert_eq!(
    outline.follow(&search),
    Some(Follow::Link { target, description: None })
  );
  let target = Target::Uri("https://example.org".into());
  let outline = panel(PanelKind::Outline, Destination::Link(target.clone()));
  assert_eq!(
    outline.follow(&search),
    Some(Follow::Link { target, description: None })
  );
}

#[test]
fn refocus_reuses_destinations() {
  let position = Position::default();
  let note = annotation();
  let notes = panel(
    PanelKind::Annotations,
    Destination::Annotation { position, annotation: note.clone() },
  );
  assert_eq!(
    notes.follow(&Search::default()),
    Some(Follow::Annotation { position, annotation: note.clone() })
  );
  assert_eq!(
    notes.refocus(),
    Some(Follow::Focus(Focus::Annotation { position, annotation: note }))
  );
  let mark = panel(PanelKind::Marks, Destination::Mark('A'));
  assert_eq!(mark.refocus(), Some(Follow::Mark('A')));
  let file = panel(PanelKind::Recent, Destination::File("paper.pdf".into()));
  assert_eq!(
    file.follow(&Search::default()),
    Some(Follow::File("paper.pdf".into()))
  );
  assert_eq!(file.refocus(), Some(Follow::Focus(Focus::Content)));
  let empty = panel(PanelKind::Help, Destination::None);
  assert_eq!(empty.refocus(), None);
  assert_eq!(empty.follow(&Search::default()), None);
}

fn mouse(kind: MouseEventKind, row: u16) -> MouseEvent {
  MouseEvent { kind, column: 2, row, modifiers: KeyModifiers::NONE }
}

fn activate(panel: &mut Panel, mouse: MouseEvent, clicks: u8) -> Activation {
  let press = panel.press(mouse);
  press.activate(panel, clicks)
}

#[test]
fn presses_preserve_click_history() {
  let left = MouseEventKind::Down(MouseButton::Left);
  let mut view = panel(PanelKind::Outline, Destination::None);
  assert_eq!(
    activate(&mut view, mouse(left, 1), 1),
    Activation::Follow { focus: false }
  );
  assert_eq!(
    activate(&mut view, mouse(MouseEventKind::ScrollUp, 1), 1),
    Activation::Stay
  );
  assert_eq!(
    activate(&mut view, mouse(MouseEventKind::Down(MouseButton::Right), 1), 1),
    Activation::Context
  );
  assert_eq!(activate(&mut view, mouse(left, 1), 2), Activation::Refocus);
  assert_eq!(activate(&mut view, mouse(left, 0), 1), Activation::Stay);
  assert_eq!(
    activate(&mut view, mouse(left, 1), 2),
    Activation::Follow { focus: false }
  );
  let mut recent = Panel::recent(vec![Row::parse("document")], &[]);
  recent.focus(false);
  assert_eq!(
    activate(&mut recent, mouse(left, 1), 1),
    Activation::Follow { focus: false }
  );
  assert_eq!(
    activate(&mut recent, mouse(left, 1), 2),
    Activation::Follow { focus: true }
  );
}

#[test]
fn activation_follows_relayout() -> Result<()> {
  let left = MouseEventKind::Down(MouseButton::Left);
  let mut view = panel(PanelKind::Outline, Destination::None);
  let _interrupted = view.press(mouse(left, 1));
  assert_eq!(
    activate(&mut view, mouse(left, 1), 2),
    Activation::Follow { focus: false }
  );
  let middle = MouseEvent {
    column: 20,
    ..mouse(MouseEventKind::Down(MouseButton::Middle), 1)
  };
  let press = view.press(middle);
  view.list(&mut Vec::new(), vpdf_model::geometry::At::default(), 12, 8)?;
  assert_eq!(press.activate(&mut view, 1), Activation::Stay);
  assert_eq!(
    activate(&mut view, mouse(MouseEventKind::Down(MouseButton::Middle), 1), 1),
    Activation::Split(0)
  );
  Ok(())
}

#[test]
fn focus_ends_selection() {
  let mut reader = Pane::default();
  for note in [false, true] {
    reader.enter_cursor();
    reader.visual(SelectionKind::Character);
    assert!(reader.selection.is_some());
    let position = Position::default();
    let focus = if note {
      Focus::Annotation { position, annotation: annotation() }
    } else {
      Focus::Content
    };
    assert_eq!(focus.apply(&mut reader).is_some(), note);
    assert!(reader.selection.is_none());
    assert_eq!(reader.mode(), if note { Mode::Cursor } else { Mode::Normal });
  }
}
