use crate::common;
use crate::harness::{corpus, solo, temp_copy};
use vpdf::Result;
use vpdf::app::App;
use vpdf::jobs::Loading;
use vpdf::session::Session;
use vpdf::solo::Solo;
use vpdf_model::text::PageId;
use vpdf_pdf::document::Handle;
use vpdf_ui::keys::{Action, Input, PanelKind};
use vpdf_ui::layout::Screen;

fn key(title: &str) -> String {
  title
    .chars()
    .filter(|c| c.is_alphanumeric())
    .collect::<String>()
    .to_lowercase()
}

/// Inferred headings against each corpus PDF's real outline.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn inferred_outline_matches_authored_one() -> Result<()> {
  for (name, recall_floor, precision_floor) in
    [("paper.pdf", 0.95, 0.75), ("book.pdf", 0.95, 0.7)]
  {
    // The text the inference worker reads.
    let handle = Handle::open(&corpus(name)?, "")?;
    let pages = (0..handle.pages)
      .map(|page| {
        handle
          .text(PageId(page))
          .map(|t| vpdf_pdf::jobs::outline::summarize(&t))
      })
      .collect::<Result<Vec<_>>>()?;
    let inferred = vpdf_pdf::jobs::outline::infer(&pages);
    let authored = handle.outline()?;
    let real: Vec<_> = authored.iter().map(|h| key(&h.title)).collect();
    let guessed: Vec<_> = inferred.iter().map(|h| key(&h.title)).collect();
    let found = real
      .iter()
      .filter(|r| guessed.iter().any(|g| g.ends_with(r.as_str())))
      .count();
    let right = guessed
      .iter()
      .filter(|g| real.iter().any(|r| g.ends_with(r.as_str())))
      .count();
    let recall = found as f32 / real.len() as f32;
    let precision = right as f32 / guessed.len().max(1) as f32;
    eprintln!(
      "{name}: recall {recall:.2} precision {precision:.2} ({} guessed)",
      guessed.len()
    );
    for h in inferred
      .iter()
      .filter(|h| !real.iter().any(|r| key(&h.title).ends_with(r.as_str())))
      .take(15)
    {
      eprintln!("  extra: {} {}", h.depth, h.title);
    }
    for r in authored
      .iter()
      .filter(|r| !guessed.iter().any(|g| g.ends_with(&key(&r.title))))
      .take(15)
    {
      eprintln!("  missed: {}", r.title);
    }
    assert!(recall >= recall_floor && precision >= precision_floor, "{name}");
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn documents_without_outline_infer_one_in_background() -> Result<()> {
  let mut app = solo("form.pdf", false, SCREEN)?;
  assert!(matches!(app.outline_load, Loading::Deferred));
  app.want_outline();
  common::wait_until(10, "Outline", || app.outline_updates())?;
  assert!(app.doc.inferred);
  let mut book = solo("book.pdf", false, SCREEN)?;
  book.want_outline();
  common::wait_until(10, "Outline", || book.outline_updates())?;
  assert!(!book.doc.inferred && !book.doc.outline.is_empty(), "authored");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn inferred_outline_keeps_panel_focus_and_selection() -> Result<()> {
  let (_directory, path) = temp_copy("form.pdf")?;
  let mut app = Solo::open(&path, "", false, SCREEN)?;
  let finish = |app: &mut App| -> Result<()> {
    common::wait_until(10, "Outline", || app.outline_updates())
  };
  // Opened while inferring: rows arrive, and the page keeps the keyboard.
  app.open_panel(PanelKind::Outline)?;
  app.panel.as_mut().unwrap().view.focus(false);
  finish(&mut app)?;
  let panel = app.panel.as_ref().unwrap();
  assert!(
    panel.view.rows().len() > 1
      && panel.view.note() == "inferred"
      && !panel.view.focused()
  );
  // Reload invalidates indices and preserves focus.
  for focused in [true, false] {
    let panel = app.panel.as_mut().unwrap();
    panel.view.focus(focused);
    panel.view.choose(panel.view.rows().len() - 1);
    app.command("reload")?;
    finish(&mut app)?;
    let selected = if focused { 0 } else { app.outline.current().unwrap_or(0) };
    let panel = app.panel.as_ref().unwrap();
    assert_eq!(
      (panel.view.focused(), panel.view.selected()),
      (focused, selected)
    );
    assert_eq!(panel.view.note(), "inferred");
  }
  // A rebuild that ships bookmarks drops the note once they load.
  std::fs::copy(corpus("book.pdf")?, &path)?;
  app.command("reload")?;
  finish(&mut app)?;
  let panel = app.panel.as_ref().unwrap();
  assert!(!panel.view.rows().is_empty() && panel.view.note().is_empty());
  Ok(())
}

const SCREEN: Screen = common::cells(120, 50);

fn deferred(app: &App) -> bool {
  matches!(app.outline_load, Loading::Deferred)
}

fn running(app: &App) -> bool {
  matches!(app.outline_load, Loading::Running(_))
}

/// Automatic loading starts after a frame shows the shown pane's whole
/// current view; a hidden tab's pane keeps waiting.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn outline_waits_for_the_whole_first_view() -> Result<()> {
  use vpdf::trace::Drawable;
  let mut session = crate::harness::session(&corpus("paper.pdf")?, SCREEN)?;
  session.open(&corpus("book.pdf")?, true, SCREEN)?;
  let frame = |session: &mut Session| -> Result<()> {
    session.draw_frame(&mut Vec::new())?;
    session.drawn();
    Ok(())
  };
  // No tile has arrived, so the first frame shows no whole view.
  frame(&mut session)?;
  assert!(!session.app().complete && deferred(session.app()));
  // The view moves before its tiles arrive; the new view governs.
  session.app().command("30")?;
  common::wait_until(20, "Tiles", || {
    assert!(deferred(session.app()));
    let tab = &mut session.tabs[session.active];
    tab.panes[tab.focus].raster_updates(&mut session.graphics);
    frame(&mut session)?;
    Ok(session.app().complete)
  })?;
  assert_eq!(session.app().reader.cursor.page.0, 29);
  assert!(running(session.app()));
  assert!(deferred(&session.tabs[0].panes[0]), "hidden tab waits");
  // Overview thumbnails count like tiles; a covered pane never completes.
  let mut app = solo("paper.pdf", false, SCREEN)?;
  app.open_overview()?;
  app.draw(&mut Vec::new())?;
  assert!(!app.complete, "no thumbnail yet");
  app.close_overview();
  app.show_recent(&[])?;
  app.draw(&mut Vec::new())?;
  assert!(app.covered() && !app.complete);
  // Text views have no tiles to wait for.
  let mut text = solo("paper.pdf", true, SCREEN)?;
  text.draw(&mut Vec::new())?;
  text.start_deferred();
  assert!(running(&text));
  Ok(())
}

/// Commands that need the outline start it at once; `:cancel` stops
/// queued and running work alike, and a reload defers it again.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn outline_requests_and_cancels_override_deferral() -> Result<()> {
  let path = corpus("paper.pdf")?;
  let mut app = Solo::open(&path, "", false, SCREEN)?;
  app.command("cancel")?;
  assert!(matches!(app.outline_load, Loading::Canceled));
  app.complete = true;
  app.start_deferred();
  assert!(
    matches!(app.outline_load, Loading::Canceled),
    "no restart per frame"
  );
  app.open_panel(PanelKind::Outline)?;
  assert!(running(&app), "explicit request restarts");
  app.command("cancel")?;
  assert!(matches!(app.outline_load, Loading::Canceled));
  app.close_panel()?;
  app.command("reload")?;
  assert!(deferred(&app));
  app.load_outline();
  app.command("reload")?;
  assert!(deferred(&app), "reload drops the running job");
  // A section motion before any tile starts loading.
  let mut section = Solo::open(&path, "", false, SCREEN)?;
  let input = Input {
    action: Action::Section(true),
    count: 1,
    explicit: false,
    register: '"',
  };
  section.action(input)?;
  assert!(running(&section));
  common::outline_loaded(&mut section)?;
  assert!(matches!(section.outline_load, Loading::Loaded));
  assert!(!section.doc.outline.is_empty());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn contents_auto_survives_delivery_and_reopening() -> Result<()> {
  use vpdf_ui::keys::Fold;
  for name in ["paper.pdf", "book.pdf"] {
    let mut app = solo(name, false, SCREEN)?;
    app.open_panel(PanelKind::Outline)?;
    assert!(app.panel.as_ref().unwrap().view.rows().is_empty());
    common::keys(&mut app, b"za")?;
    app.open_panel(PanelKind::Version)?;
    common::outline_loaded(&mut app)?;
    app.open_panel(PanelKind::Outline)?;
    assert!(app.panel.as_ref().unwrap().view.auto_expand());
    let branch = app
      .outline
      .tree()
      .nodes()
      .iter()
      .position(|node| node.descendants > 0)
      .unwrap();
    let target = app.doc.outline[branch + 1].target.clone();
    app.close_panel()?;
    app.follow(&target)?;
    app.track_outline();
    app.open_panel(PanelKind::Outline)?;
    let current = app.outline.current().unwrap();
    let mut path = vec![current];
    let mut parent = app.outline.tree().nodes()[current].parent;
    while let Some(source) = parent {
      path.push(source);
      parent = app.outline.tree().nodes()[source].parent;
    }
    for (source, node) in app.outline.tree().nodes().iter().enumerate() {
      assert_eq!(
        app.outline.closed(source),
        node.descendants > 0 && !path.contains(&source),
        "{name} {source}"
      );
    }
    assert!(app.panel.as_ref().unwrap().view.auto_expand());
    app.refill_panel(PanelKind::Outline)?;
    assert_eq!(app.outline.current(), Some(current));
    assert!(app.panel.as_ref().unwrap().view.auto_expand());
    common::keys(&mut app, b"za")?;
    assert!(!app.panel.as_ref().unwrap().view.auto_expand());
    app.panel_input(Input::plain(Action::Fold(Fold::CollapseAll)))?;
    app.close_panel()?;
    app.open_panel(PanelKind::Outline)?;
    assert!(app.outline.closed(branch));
    assert!(matches!(app.outline_load, Loading::Loaded));
    common::keys(&mut app, b"za")?;
    app.command("reload")?;
    common::outline_loaded(&mut app)?;
    assert!(app.panel.as_ref().unwrap().view.auto_expand());
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn contents_search_and_tracking_lifecycle() -> Result<()> {
  for name in ["paper.pdf", "book.pdf"] {
    let mut app = solo(name, false, SCREEN)?;
    common::outline_loaded(&mut app)?;
    app.open_panel(PanelKind::Outline)?;
    let source = app
      .doc
      .outline
      .iter()
      .enumerate()
      .position(|(source, heading)| {
        app.outline.tree().nodes()[source].parent.is_some()
          && app
            .doc
            .outline
            .iter()
            .filter(|other| other.title == heading.title)
            .count()
            == 1
      })
      .unwrap();
    let title = app.doc.outline[source].title.clone();
    common::keys(&mut app, b"zM")?;
    let cursor = app.reader.cursor;
    let selected = app.panel.as_ref().unwrap().view.selected();
    let top = app.panel.as_ref().unwrap().view.top();
    app.begin_search(false)?;
    app.incremental(&title)?;
    app.draw(&mut Vec::new())?;
    assert_eq!(app.panel.as_ref().unwrap().view.selected(), source);
    assert_eq!(app.panel.as_ref().unwrap().view.count(), Some((1, 1)));
    app.abort_search()?;
    let panel = &app.panel.as_ref().unwrap().view;
    assert_eq!((panel.selected(), panel.top()), (selected, top));
    app.cancel_prompt()?;
    app.begin_search(false)?;
    app.incremental(&title)?;
    app.finish_search(&title, false)?;
    app.cancel_prompt()?;
    assert_eq!(app.panel.as_ref().unwrap().view.selected(), source);
    assert_eq!(app.reader.cursor, cursor);
    common::keys(&mut app, b"zM")?;
    common::keys(&mut app, b"n")?;
    assert_eq!(app.panel.as_ref().unwrap().view.selected(), source);
    assert!(!app.panel.as_ref().unwrap().view.auto_expand());
    common::keys(&mut app, b"zM")?;
    let selected = app.panel.as_ref().unwrap().view.selected();
    let top = app.panel.as_ref().unwrap().view.top();
    let target = app.doc.outline[source].target.clone();
    app.follow(&target)?;
    app.track_outline();
    assert_eq!(app.panel.as_ref().unwrap().view.selected(), selected);
    assert_eq!(app.panel.as_ref().unwrap().view.top(), top);
    assert!(
      app.outline.closed(app.outline.tree().nodes()[source].parent.unwrap())
    );
    app.panel.as_mut().unwrap().view.focus(false);
    app.track_outline();
    let current = app.outline.current().unwrap();
    let mut ancestor = current;
    while let Some(parent) = app.outline.tree().nodes()[ancestor].parent {
      ancestor = parent;
    }
    assert_eq!(app.panel.as_ref().unwrap().view.selected(), ancestor);
    app.open_panel(PanelKind::Outline)?;
    common::keys(&mut app, b"za")?;
    assert!(app.panel.as_ref().unwrap().view.auto_expand());
    app.begin_search(false)?;
    app.incremental(&title)?;
    app.abort_search()?;
    app.cancel_prompt()?;
    assert!(app.panel.as_ref().unwrap().view.auto_expand());
    app.begin_search(false)?;
    assert!(app.finish_search("absent-outline-phrase", false).is_err());
    app.cancel_prompt()?;
    assert!(app.panel.as_ref().unwrap().view.auto_expand());
    app.begin_search(false)?;
    app.finish_search(&title, false)?;
    app.cancel_prompt()?;
    assert!(!app.panel.as_ref().unwrap().view.auto_expand());
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn contents_panes_browse_independently() -> Result<()> {
  let mut session = crate::harness::session(&corpus("paper.pdf")?, SCREEN)?;
  common::outline_loaded(session.app())?;
  session.app().open_panel(PanelKind::Outline)?;
  common::keys(session.app(), b"zM")?;
  let branch = session
    .app()
    .outline
    .tree()
    .nodes()
    .iter()
    .position(|node| node.descendants > 0)
    .unwrap();
  session.app().command("vsplit")?;
  session.settle()?;
  common::outline_loaded(session.app())?;
  session.app().open_panel(PanelKind::Outline)?;
  common::keys(session.app(), b"zRza")?;
  let panes = &session.tabs[session.active].panes;
  assert_eq!(panes.len(), 2);
  assert!(panes[0].outline.closed(branch));
  assert!(!panes[0].panel.as_ref().unwrap().view.auto_expand());
  assert!(panes[1].panel.as_ref().unwrap().view.auto_expand());
  Ok(())
}
