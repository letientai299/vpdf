use std::path::PathBuf;
use vpdf::Result;
use vpdf::app::App;
use vpdf::document::Handle;
use vpdf::outline::Loading;
use vpdf::session::Session;
use vpdf::solo::Solo;
use vpdf::text::PageId;
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
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  for (name, recall_floor, precision_floor) in
    [("paper.pdf", 0.95, 0.75), ("book.pdf", 0.95, 0.7)]
  {
    // The text the inference worker reads.
    let handle = Handle::open(&root.join(name), "")?;
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
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut app = Solo::open(&root.join("form.pdf"), "", false, SCREEN)?;
  assert!(matches!(app.outline_load, Loading::Deferred));
  app.want_outline();
  let started = std::time::Instant::now();
  while !app.outline_updates()? {
    assert!(started.elapsed().as_secs() < 10);
    std::thread::sleep(std::time::Duration::from_millis(5));
  }
  assert!(app.doc.inferred);
  let mut book = Solo::open(&root.join("book.pdf"), "", false, SCREEN)?;
  book.want_outline();
  while !book.outline_updates()? {
    assert!(started.elapsed().as_secs() < 10);
    std::thread::sleep(std::time::Duration::from_millis(5));
  }
  assert!(!book.doc.inferred && !book.doc.outline.is_empty(), "authored");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn inferred_outline_keeps_panel_focus_and_selection() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("live.pdf");
  std::fs::copy(root.join("form.pdf"), &path)?;
  let mut app = Solo::open(&path, "", false, SCREEN)?;
  let finish = |app: &mut App| -> Result<()> {
    let started = std::time::Instant::now();
    while !app.outline_updates()? {
      assert!(started.elapsed().as_secs() < 10);
      std::thread::sleep(std::time::Duration::from_millis(5));
    }
    Ok(())
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
  // A reload infers again under the user's focus and selection.
  for focused in [true, false] {
    let panel = app.panel.as_mut().unwrap();
    panel.view.focus(focused);
    panel.view.choose(panel.view.rows().len() - 1);
    let selected = panel.view.selected();
    app.command("reload")?;
    finish(&mut app)?;
    let panel = app.panel.as_ref().unwrap();
    assert_eq!(
      (panel.view.focused(), panel.view.selected()),
      (focused, selected)
    );
    assert_eq!(panel.view.note(), "inferred");
  }
  // A rebuild that ships bookmarks drops the note once they load.
  std::fs::copy(root.join("book.pdf"), &path)?;
  app.command("reload")?;
  finish(&mut app)?;
  let panel = app.panel.as_ref().unwrap();
  assert!(!panel.view.rows().is_empty() && panel.view.note().is_empty());
  Ok(())
}

const SCREEN: Screen = Screen {
  columns: 120,
  rows: 50,
  cell_width: 8,
  cell_height: 16,
  origin: (0, 0),
};

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
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut session = Session::new(None);
  session.open(&root.join("paper.pdf"), true, SCREEN)?;
  session.open(&root.join("book.pdf"), true, SCREEN)?;
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
  let started = std::time::Instant::now();
  while !session.app().complete {
    assert!(started.elapsed().as_secs() < 20, "tiles render");
    assert!(deferred(session.app()));
    let tab = &mut session.tabs[session.active];
    tab.panes[tab.focus].raster_updates(&mut session.graphics);
    frame(&mut session)?;
    std::thread::sleep(std::time::Duration::from_millis(2));
  }
  assert_eq!(session.app().reader.cursor.page.0, 29);
  assert!(running(session.app()));
  assert!(deferred(&session.tabs[0].panes[0]), "hidden tab waits");
  // Overview thumbnails count like tiles; a covered pane never completes.
  let mut app = Solo::open(&root.join("paper.pdf"), "", false, SCREEN)?;
  app.open_overview()?;
  app.draw(&mut Vec::new())?;
  assert!(!app.complete, "no thumbnail yet");
  app.close_overview();
  app.show_recent(&[])?;
  app.draw(&mut Vec::new())?;
  assert!(app.covered() && !app.complete);
  // Text views have no tiles to wait for.
  let mut text = Solo::open(&root.join("paper.pdf"), "", true, SCREEN)?;
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
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let path = root.join("paper.pdf");
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
  super::common::outline_loaded(&mut section)?;
  assert!(matches!(section.outline_load, Loading::Loaded));
  assert!(!section.doc.outline.is_empty());
  Ok(())
}
