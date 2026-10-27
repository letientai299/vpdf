use crate::harness::{corpus, solo};
use vpdf::Result;
use vpdf_model::text::PageId;

#[test]
fn raw_figure_bindings() {
  use std::collections::VecDeque;
  use vpdf_terminal::decode::{Event, Parser};
  use vpdf_ui::keys::{Action, Grammar, Keys, PanelKind};
  for (bytes, expected) in [
    (&b"]f"[..], Action::Figure(true)),
    (&b"[f"[..], Action::Figure(false)),
    (&b"zF"[..], Action::FigureFit),
    (&b"gF"[..], Action::Region),
    (&b"R"[..], Action::RegionStart),
    (&b"gf"[..], Action::Panel(PanelKind::Figures)),
    (&b"f"[..], Action::Hints(false)),
  ] {
    let mut events = VecDeque::new();
    Parser::default().feed(bytes, &mut events);
    let mut keys = Keys::default();
    let mut actions = Vec::new();
    for event in events {
      let Event::Key(key) = event else { panic!("key bytes") };
      if let Some(input) = keys.feed(key, Grammar::Normal) {
        actions.push(input.action);
      }
    }
    assert_eq!(actions, [expected], "{bytes:?}");
  }
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn real_figure_navigation_fit() -> Result<()> {
  use vpdf_ui::keys::{Action, Input};
  use vpdf_ui::region::Region;
  let screen = super::focus::screen();
  let mut app = solo("paper.pdf", false, screen)?;
  app.action(Input::plain(Action::Cursor))?;
  app.move_figure(true, 1)?;
  let first = app.overlays.region().unwrap().snapshot().clone();
  assert_ne!(first.page, PageId(0), "Title rule remained: {first:?}");
  app.move_figure(true, 1)?;
  let second = app.overlays.region().unwrap().snapshot().clone();
  assert!(
    second.page > first.page
      || second.bounds.y0 > first.bounds.y0
      || second.bounds.x0 > first.bounds.x0
  );
  app.move_figure(false, 1)?;
  let returned = app.overlays.region().unwrap().snapshot();
  assert_eq!(returned.page, first.page);
  assert_eq!(returned.bounds.y0, first.bounds.y0);
  assert_eq!(returned.bounds.x0, first.bounds.x0);
  app.fit_figure()?;
  let rect = app.layout.project(first.page, first.bounds, app.reader.scroll);
  assert!(rect.width() <= screen.width() + 1.0, "{rect:?}");
  assert!(rect.height() <= screen.height() + 1.0, "{rect:?}");
  assert!(
    rect.y1 <= screen.height() - f32::from(screen.cell_height) + 1.0,
    "Action bar overlaps figure: {rect:?}"
  );
  app.reader.visual(vpdf_reader::reader::SelectionKind::Character);
  let selection = app.reader.selection;
  let cursor = app.reader.cursor;
  let mode = app.reader.mode();
  app.action(Input::plain(Action::Region))?;
  assert_eq!(
    app.overlays.region().and_then(Region::action),
    Some(vpdf_ui::region::Operation::Image)
  );
  assert_eq!(app.reader.mode(), mode);
  assert_eq!(app.reader.cursor, cursor);
  assert_eq!(
    app.reader.selection.map(|range| (range.anchor, range.kind)),
    selection.map(|range| (range.anchor, range.kind))
  );
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn raw_region_preserves_state() -> Result<()> {
  use super::common::keys;
  use super::modes::app;
  use vpdf_reader::reader::Mode;
  use vpdf_ui::input::InputOwner;
  use vpdf_ui::region::Operation;
  use vpdf_ui::region::Region;
  let mut app = app("paper.pdf", false)?;
  keys(&mut app, b"i]fvgF")?;
  let mode = app.reader.mode();
  assert!(matches!(mode, Mode::Visual(_)));
  let cursor = app.reader.cursor;
  let selection = app.reader.selection.map(|range| (range.anchor, range.kind));
  let scroll = app.reader.scroll;
  let capture = app.overlays.region().unwrap().snapshot().clone();
  keys(&mut app, b"hjkl")?;
  assert_eq!(
    app.overlays.region().and_then(Region::action),
    Some(Operation::Image)
  );
  assert_eq!(app.reader.cursor, cursor);
  assert_eq!(app.reader.scroll, scroll);
  keys(&mut app, b"g")?;
  assert_eq!(app.input_owner(), InputOwner::Region);
  keys(&mut app, b"\x1b[27u")?;
  assert_eq!(app.input_owner(), InputOwner::Region);
  assert_eq!(app.keys.pending(), "");
  keys(&mut app, b":\x1b[27u")?;
  assert_eq!(app.input_owner(), InputOwner::Region);
  assert_eq!(app.overlays.region().unwrap().snapshot().id, capture.id);
  keys(&mut app, b":\r")?;
  assert_eq!(app.input_owner(), InputOwner::Region);
  assert!(keys(&mut app, b":unknown-region-command\r").is_err());
  assert_eq!(app.input_owner(), InputOwner::Region);
  keys(&mut app, b"\x1b[27u")?;
  assert_eq!(app.reader.mode(), mode);
  assert_eq!(app.reader.cursor, cursor);
  assert_eq!(
    app.reader.selection.map(|range| (range.anchor, range.kind)),
    selection
  );
  keys(&mut app, b"\x1b[27u")?;
  assert_eq!(app.reader.mode(), Mode::Cursor);
  keys(&mut app, b"\x1b[27u")?;
  assert_eq!(app.reader.mode(), Mode::Normal);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn raw_region_pointer_transfer() -> Result<()> {
  use super::focus::{screen, send};
  use vpdf_reader::reader::Mode;
  use vpdf_ui::input::InputOwner;
  let mut session = crate::harness::session(&corpus("paper.pdf")?, screen())?;
  send(&mut session, b"i]fvgF")?;
  let mode = session.app().unwrap().reader.mode();
  let cursor = session.app().unwrap().reader.cursor;
  let scroll = session.app().unwrap().reader.scroll;
  send(&mut session, b"\x1b[<65;60;8M")?;
  assert_eq!(session.app().unwrap().input_owner(), InputOwner::Region);
  assert_eq!(session.app().unwrap().reader.scroll, scroll);
  send(&mut session, b"\x1b[<0;60;8M\x1b[<0;60;8m")?;
  assert_eq!(session.app().unwrap().input_owner(), InputOwner::Content);
  assert_eq!(session.app().unwrap().reader.mode(), mode);
  assert_eq!(session.app().unwrap().reader.cursor, cursor);
  send(&mut session, b"gF\x17")?;
  assert_eq!(session.app().unwrap().input_owner(), InputOwner::Content);
  assert_eq!(session.app().unwrap().keys.awaiting(), Some("C-w"));
  send(&mut session, b"v")?;
  assert_eq!(session.tab().panes.len(), 2);
  assert_eq!(session.app().unwrap().reader.mode(), Mode::Normal);
  send(&mut session, b"\x17p")?;
  assert_eq!(session.app().unwrap().reader.mode(), mode);
  assert_eq!(session.app().unwrap().reader.cursor, cursor);
  assert!(session.app().unwrap().overlays.region().is_none());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn real_figure_sidebar() -> Result<()> {
  use super::common::keys;
  use vpdf_ui::keys::PanelKind;
  let mut app = solo("paper.pdf", false, super::focus::screen())?;
  keys(&mut app, b"gf")?;
  assert_eq!(app.panel.as_ref().unwrap().view.kind(), PanelKind::Figures);
  assert_eq!(app.panel.as_ref().unwrap().view.note(), "loading");
  figures_loaded(&mut app)?;
  let expected = figure_rows(&mut app)?;
  let index = expected.iter().position(|(page, _)| page.0 > 0).unwrap();
  let page = expected[index].0;
  app.panel.as_mut().unwrap().view.select(index);
  keys(&mut app, b"o")?;
  assert!(app.panel_focused());
  assert_eq!(app.navigation_position().page, page);
  // The marker follows the page, not the selection.
  app.panel.as_mut().unwrap().view.choose(index + 1);
  app.track_figures();
  assert_eq!(app.panel.as_ref().unwrap().view.current(), Some(index));
  app.go_to_page(PageId(0))?;
  app.track_figures();
  let before = expected.iter().position(|(at, _)| at.0 == 0);
  assert_eq!(app.panel.as_ref().unwrap().view.current(), before);
  let bare = (page.0 + 1..app.doc.pages.len())
    .map(PageId)
    .find(|at| expected.iter().all(|(page, _)| page != at))
    .expect("page without figures");
  app.go_to_page(bare)?;
  app.track_figures();
  let before = expected.iter().rposition(|(at, _)| *at < bare);
  assert_eq!(app.panel.as_ref().unwrap().view.current(), before);
  keys(&mut app, b"\r")?;
  assert!(!app.panel_focused());
  keys(&mut app, b"gqgf")?;
  assert_eq!(app.panel.as_ref().unwrap().view.rows().len(), expected.len());
  assert_eq!(app.panel.as_ref().unwrap().view.note(), "");
  app.reload_with("")?;
  assert!(matches!(app.figures, vpdf::jobs::Figures::Running(_)));
  assert!(app.panel.as_ref().unwrap().view.rows().is_empty());
  keys(&mut app, b":cancel\r")?;
  assert!(matches!(app.figures, vpdf::jobs::Figures::Idle));
  assert_eq!(app.panel.as_ref().unwrap().view.note(), "");
  keys(&mut app, b"gf")?;
  assert!(app.panel.is_none());
  keys(&mut app, b"gf")?;
  figures_loaded(&mut app)?;
  assert_eq!(app.panel.as_ref().unwrap().view.rows().len(), expected.len());
  Ok(())
}

fn figures_loaded(app: &mut vpdf::app::App) -> Result<()> {
  crate::common::wait_until(20, "Figure scan", || {
    app.figure_updates()?;
    Ok(!matches!(app.figures, vpdf::jobs::Figures::Running(_)))
  })?;
  anyhow::ensure!(app.error.is_none(), "{:?}", app.error);
  Ok(())
}

fn figure_rows(
  app: &mut vpdf::app::App,
) -> Result<vpdf_pdf::jobs::figures::Entries> {
  use vpdf_model::document::Target;
  use vpdf_ui::panel::{Destination, RowKind};
  let mut expected = Vec::new();
  for page in 0..app.doc.pages.len() {
    for figure in app.doc.figures(PageId(page))?.iter() {
      expected.push((PageId(page), figure.clone()));
    }
  }
  if expected.iter().any(|(_, figure)| !figure.caption.is_empty()) {
    expected.retain(|(_, figure)| !figure.caption.is_empty());
  }
  let panel = &app.panel.as_ref().unwrap().view;
  assert!(!panel.rows().is_empty());
  assert_eq!(panel.rows().len(), expected.len());
  assert_eq!(panel.note(), "");
  for (row, (page, figure)) in panel.rows().iter().zip(&expected) {
    assert!(
      row.destination
        == Destination::Link(Target::Point {
          page: *page,
          x: figure.bounds.x0,
          y: figure.bounds.y0,
        })
    );
    assert_eq!(row.detail, vpdf_ui::panel::Row::page(app.doc.label(*page)));
    let caption = figure.caption.split_whitespace().collect::<Vec<_>>();
    let caption = caption.join(" ");
    match vpdf_model::caption::parse(&caption) {
      Some(parsed) => {
        assert_eq!(row.label, parsed.rest, "{caption}");
        assert!(row.key.ends_with(&format!(" {}", parsed.number)));
      }
      None if caption.is_empty() => assert_eq!(row.key, ""),
      None => assert_eq!(row.kind, RowKind::Mention, "{caption}"),
    }
    assert!(!caption.contains("summarizes our results"), "{caption}");
  }
  Ok(expected)
}
