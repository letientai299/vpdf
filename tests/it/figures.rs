use std::path::PathBuf;
use vpdf::Result;
use vpdf::solo::Solo;
use vpdf_model::text::PageId;

#[test]
fn raw_figure_bindings() {
  use std::collections::VecDeque;
  use vpdf_terminal::decode::{Event, Parser};
  use vpdf_ui::keys::{Action, Grammar, Keys, PanelKind};
  for (bytes, expected) in [
    (&b"]i"[..], Action::Figure(true)),
    (&b"[i"[..], Action::Figure(false)),
    (&b"zF"[..], Action::FigureFit),
    (&b"gF"[..], Action::Region),
    (&b"R"[..], Action::RegionStart),
    (&b"gf"[..], Action::Panel(PanelKind::Figures)),
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
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = super::focus::screen();
  let mut app = Solo::open(&root.join("paper.pdf"), "", false, screen)?;
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
  keys(&mut app, b"i]ivgF")?;
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
  use vpdf::session::Session;
  use vpdf_reader::reader::Mode;
  use vpdf_ui::input::InputOwner;
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut session = Session::new(None);
  session.open(&root.join("paper.pdf"), true, screen())?;
  send(&mut session, b"i]ivgF")?;
  let mode = session.app().reader.mode();
  let cursor = session.app().reader.cursor;
  let scroll = session.app().reader.scroll;
  send(&mut session, b"\x1b[<65;60;8M")?;
  assert_eq!(session.app().input_owner(), InputOwner::Region);
  assert_eq!(session.app().reader.scroll, scroll);
  send(&mut session, b"\x1b[<0;60;8M\x1b[<0;60;8m")?;
  assert_eq!(session.app().input_owner(), InputOwner::Content);
  assert_eq!(session.app().reader.mode(), mode);
  assert_eq!(session.app().reader.cursor, cursor);
  send(&mut session, b"gF\x17")?;
  assert_eq!(session.app().input_owner(), InputOwner::Content);
  assert_eq!(session.app().keys.awaiting(), Some("C-w"));
  send(&mut session, b"v")?;
  assert_eq!(session.tab().panes.len(), 2);
  assert_eq!(session.app().reader.mode(), Mode::Normal);
  send(&mut session, b"\x17p")?;
  assert_eq!(session.app().reader.mode(), mode);
  assert_eq!(session.app().reader.cursor, cursor);
  assert!(session.app().overlays.region().is_none());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn real_figure_sidebar() -> Result<()> {
  use super::common::keys;
  use vpdf_ui::keys::PanelKind;

  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut app =
    Solo::open(&root.join("paper.pdf"), "", false, super::focus::screen())?;
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
  use std::time::{Duration, Instant};
  let deadline = Instant::now() + Duration::from_secs(20);
  while matches!(app.figures, vpdf::jobs::Figures::Running(_)) {
    app.figure_updates()?;
    anyhow::ensure!(Instant::now() < deadline, "Figure scan timed out");
    std::thread::sleep(Duration::from_millis(5));
  }
  anyhow::ensure!(app.error.is_none(), "{:?}", app.error);
  Ok(())
}

fn figure_rows(
  app: &mut vpdf::app::App,
) -> Result<vpdf_pdf::jobs::figures::Entries> {
  use vpdf_model::document::Target;
  use vpdf_ui::panel::Destination;
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
    assert_eq!(row.detail, app.doc.label(*page));
    if !figure.caption.trim().is_empty() {
      assert_eq!(
        row.label,
        figure.caption.split_whitespace().collect::<Vec<_>>().join(" ")
      );
    }
  }
  Ok(expected)
}

#[test]
#[ignore = "requires PMPP, SDM, and PMLA figure PDFs"]
fn raw_region_at_captions() -> Result<()> {
  use super::common::keys;
  use vpdf_model::text::Position;
  for (variable, page, caption, height) in [
    ("VPDF_FIGURE_PMPP", PageId(161), "FIGURE 6.13", 380.0),
    ("VPDF_FIGURE_SDM", PageId(1149), "Figure 1-17.", 130.0),
    ("VPDF_FIGURE_PMLA", PageId(49), "Figure 2.4:", 140.0),
    ("VPDF_FIGURE_PMLA", PageId(49), "Figure 2.5:", 120.0),
    ("VPDF_FIGURE_PMLA", PageId(81), "Figure 2.15:", 90.0),
    ("VPDF_FIGURE_PMLA", PageId(82), "Figure 2.16:", 40.0),
    ("VPDF_FIGURE_PMLA", PageId(439), "Figure 9.4:", 120.0),
  ] {
    let path = PathBuf::from(std::env::var(variable)?);
    let mut app = Solo::open(&path, "", false, super::focus::screen())?;
    let text = app.doc.text(page)?;
    let line = text
      .lines
      .iter()
      .position(|line| line.text().starts_with(caption))
      .unwrap();
    keys(&mut app, b"i")?;
    app.go_to(Position { page, line, glyph: 0 }, true)?;
    keys(&mut app, b"gF")?;
    let region = app.overlays.region().unwrap().snapshot();
    assert_eq!(region.page, page);
    assert!(region.bounds.height() > height, "{region:?}");
    assert!(region.bounds.y0 > 50.0, "Header included: {region:?}");
    assert!(region.bounds.y1 < 580.0, "Body included: {region:?}");
  }
  Ok(())
}
