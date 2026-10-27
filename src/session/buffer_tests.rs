use super::*;
use crate::screen::cells;
use vpdf_model::input::{KeyCode, KeyModifiers};
use vpdf_pdf::jobs::Wake;

fn input(session: &mut Session, key: KeyEvent) {
  if matches!(session.frontend, crate::raster::Frontend::Native(_)) {
    let mut screen = session.screen.unwrap();
    native::apply(session, native::Input::Key(key), &mut screen);
  } else {
    run::apply(session, Act::Key(key), &mut Vec::new());
  }
}

fn keys(session: &mut Session, sequence: &str) -> Result<()> {
  for &key in vpdf_ui::bindings::Sequence::parse(sequence)?.keys() {
    input(session, key);
  }
  Ok(())
}

fn command(session: &mut Session, line: &str) {
  for ch in format!(":{line}").chars() {
    input(session, KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
  }
  input(session, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
}

fn session(native: bool, store: Option<PathBuf>) -> Result<Session> {
  let mut session = Session::new(store);
  if native {
    session.frontend = crate::raster::Frontend::Native(Wake::none());
  }
  session.recent_tab(cells(160, 50))?;
  Ok(session)
}

#[test]
fn buffer_transition_table() -> Result<()> {
  let root = tempfile::tempdir()?;
  let first = root.path().join("first.pdf");
  let second = root.path().join("second.pdf");
  crate::fixtures::pdf(&first, &["first", "second", "third"], "")?;
  crate::fixtures::pdf(&second, &["other"], "")?;
  let first = first.canonicalize()?;
  let second = second.canonicalize()?;
  for native in [false, true] {
    let mut session =
      session(native, Some(root.path().join(format!("state-{native}.json"))))?;
    assert_eq!(session.tabs.len(), 1);
    assert_eq!(session.tab().panes.len(), 1);
    assert!(session.focused().is_recent());
    assert!(session.command("e #").is_err());
    keys(&mut session, "<Esc>")?;
    assert!(session.running());
    command(&mut session, &format!("e {}", first.display()));
    let id = session.app()?.id;
    command(&mut session, "2");
    let position = session.app()?.navigation_position();
    for (line, document, alternate) in [
      ("recent".to_owned(), None, Some(&first)),
      ("enew".to_owned(), None, Some(&first)),
      (format!("e {}", second.display()), Some(&second), Some(&first)),
      ("e #".to_owned(), Some(&first), Some(&second)),
      ("bd".to_owned(), Some(&second), None),
      ("bd".to_owned(), None, None),
    ] {
      command(&mut session, &line);
      assert_eq!(
        session.focused().document().map(|app| &app.path),
        document,
        "{native}: {line}"
      );
      assert_eq!(
        session.focused().alternate.as_ref().map(|alt| &alt.path),
        alternate,
        "{native}: {line}"
      );
      assert_eq!(session.tabs.len(), 1);
      assert_eq!(session.tab().panes.len(), 1);
      if document == Some(&first) {
        assert_ne!(session.app()?.id, id);
        assert_eq!(session.app()?.navigation_position(), position);
      }
    }
    command(&mut session, "bd");
    assert!(!session.running());
  }
  Ok(())
}

#[test]
fn recent_dismiss_and_alternate_keys() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("book.pdf");
  crate::fixtures::pdf(&path, &["one", "two"], "")?;
  for native in [false, true] {
    for key in ["q", "<Esc>", "<C-^>", "<C-6>"] {
      let mut session = session(native, None)?;
      command(&mut session, &format!("e {}", path.display()));
      command(&mut session, "2");
      let old = session.app()?.id;
      let position = session.app()?.navigation_position();
      command(&mut session, "recent");
      assert!(session.focused().document().is_none());
      keys(&mut session, key)?;
      assert_eq!(session.app()?.path, path.canonicalize()?);
      assert_eq!(session.app()?.navigation_position(), position);
      assert_ne!(session.app()?.id, old);
      assert!(session.focused().alternate.is_none());
    }
    let mut session = session(native, None)?;
    keys(&mut session, "q")?;
    assert!(!session.running());
  }
  Ok(())
}

#[test]
fn recent_split_quits() -> Result<()> {
  for native in [false, true] {
    for quit in ["q", "q!", "x", "wq", "bd"] {
      let mut session = session(native, None)?;
      command(&mut session, "vnew");
      assert_eq!(session.tab().panes.len(), 2);
      assert!(session.tab().panes.iter().all(Pane::is_recent));
      keys(&mut session, "<C-w>l<C-w>h")?;
      command(&mut session, "new");
      assert_eq!(session.tab().panes.len(), 3);
      command(&mut session, "tabnew");
      assert_eq!(session.tabs.len(), 2);
      command(&mut session, quit);
      assert_eq!(session.tabs.len(), 1);
      assert_eq!(session.tab().panes.len(), 3);
      command(&mut session, "qa!");
      assert!(!session.running());
    }
    for quit in ["ZZ", "ZQ"] {
      let mut session = session(native, None)?;
      keys(&mut session, quit)?;
      assert!(!session.running());
    }
  }
  Ok(())
}

#[test]
fn missing_alternate_stays_and_clears() -> Result<()> {
  for native in [false, true] {
    for recent in [false, true] {
      let root = tempfile::tempdir()?;
      let first = root.path().join("first.pdf");
      let second = root.path().join("second.pdf");
      crate::fixtures::pdf(&first, &["first"], "")?;
      crate::fixtures::pdf(&second, &["second"], "")?;
      let mut session = session(native, None)?;
      session.command(&format!("e {}", first.display()))?;
      if recent {
        session.command("recent")?;
      } else {
        session.command(&format!("e {}", second.display()))?;
      }
      let id = session.focused().id;
      std::fs::remove_file(first)?;
      assert!(session.command("e #").is_err());
      assert_eq!(session.focused().id, id);
      assert!(session.focused().alternate.is_none());
      if recent {
        keys(&mut session, "q")?;
      } else {
        session.command("bd")?;
        assert!(session.focused().is_recent());
      }
    }
  }
  Ok(())
}

#[test]
fn existing_documents_focus() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("book.pdf");
  crate::fixtures::pdf(&path, &["one"], "")?;
  for native in [false, true] {
    let mut session = session(native, None)?;
    session.command(&format!("e {}", path.display()))?;
    let document = session.focused().id;
    session.command("tabnew")?;
    let recent = session.focused().id;
    session.command(&format!("e {}", path.display()))?;
    assert_eq!(session.focused().id, document);
    assert_eq!(session.tabs[1].panes[0].id, recent);
    assert!(session.tabs[1].panes[0].is_recent());
    session.command("vsplit")?;
    let peer = session.tab().panes[0].id;
    session.command("bd")?;
    assert!(session.focused().is_recent());
    assert_eq!(
      session
        .tab()
        .panes
        .iter()
        .filter(|pane| pane.document().is_some())
        .count(),
      1
    );
    assert!(session.find_pane(peer).is_some());
  }
  Ok(())
}

#[test]
fn encrypted_alternate_keeps_password() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("plain.pdf");
  crate::fixtures::pdf(&path, &["secret", "second"], "")?;
  let mut source = vpdf_pdf::document::Document::open(&path, "")?;
  let encrypted = crate::encryption::encrypt(&mut source.native)?;
  let path = encrypted.path().canonicalize()?;
  for native in [false, true] {
    let mut session = session(native, None)?;
    session.secret = Some("test-reader".into());
    session.command(&format!("e {}", path.display()))?;
    session.command("2")?;
    session.command("recent")?;
    assert!(session.focused().document().is_none());
    assert_eq!(
      session.focused().alternate.as_ref().unwrap().password,
      "test-reader"
    );
    session.command("e #")?;
    assert_eq!(session.app()?.path, path);
    assert_eq!(session.app()?.navigation_position().page, PageId(1));
    assert!(session.unlock.is_none());
  }
  Ok(())
}

#[test]
fn help_alternate_returns() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("book.pdf");
  crate::fixtures::pdf(&path, &["book"], "")?;
  for native in [false, true] {
    let mut session = session(native, None)?;
    session.command(&format!("e {}", path.display()))?;
    let book = session.focused().id;
    session.command("help navigation")?;
    let deadline = Instant::now() + Duration::from_secs(20);
    while !session.panes().any(|app| app.manual.is_some()) {
      session.help_updates(&mut io::sink());
      anyhow::ensure!(Instant::now() < deadline, "Help did not open");
      std::thread::sleep(Duration::from_millis(5));
    }
    assert!(session.app()?.manual.is_some());
    session.command("2")?;
    let position = session.app()?.navigation_position();
    session.command("recent")?;
    assert!(session.focused().document().is_none());
    session.command("e #")?;
    assert!(session.app()?.manual.is_some());
    assert_eq!(session.app()?.navigation_position(), position);
    session.activate(0)?;
    assert_eq!(session.focused().id, book);
    session.command("recent")?;
    let invoker = session.focused().id;
    session.activate(1)?;
    session.command("bd")?;
    assert_eq!(session.focused().id, invoker);
    assert!(session.focused().is_recent());
    assert_eq!(session.tabs.len(), 1);
  }
  Ok(())
}

#[test]
fn entry_preserves_alternate() -> Result<()> {
  let root = tempfile::tempdir()?;
  let first = root.path().join("first.pdf");
  let second = root.path().join("second.pdf");
  crate::fixtures::pdf(&first, &["first"], "")?;
  crate::fixtures::pdf(&second, &["second"], "")?;
  let first = first.canonicalize()?;
  let second = second.canonicalize()?;
  for native in [false, true] {
    let mut session = session(native, None)?;
    session.command(&format!("e {}", second.display()))?;
    session.command(&format!("e {}", first.display()))?;
    session.command("recent")?;
    session.draw_frame(&mut Vec::new())?;
    keys(&mut session, "j<CR>")?;
    assert_eq!(session.app()?.path, second);
    assert_eq!(session.focused().alternate.as_ref().unwrap().path, first);
    session.command("tabnew")?;
    let source = session.focused().id;
    session.draw_frame(&mut Vec::new())?;
    keys(&mut session, "<CR>")?;
    assert_eq!(session.app()?.path, second);
    assert_eq!(session.tabs[1].panes[0].id, source);
    assert!(session.tabs[1].panes[0].is_recent());
  }
  Ok(())
}

#[test]
fn external_open_destinations() -> Result<()> {
  let root = tempfile::tempdir()?;
  let first = root.path().join("first.pdf");
  let second = root.path().join("second.pdf");
  crate::fixtures::pdf(&first, &["first"], "")?;
  crate::fixtures::pdf(&second, &["second"], "")?;
  for native in [false, true] {
    let mut session = session(native, None)?;
    for (path, count) in [(&first, 1), (&second, 2), (&first, 2)] {
      if native {
        let mut screen = session.screen.unwrap();
        native::apply(
          &mut session,
          native::Input::Open(path.clone()),
          &mut screen,
        );
      } else {
        session.dropped(Target::Pdf(path.clone().into()))?;
      }
      assert_eq!(session.tabs.len(), count);
      assert_eq!(session.app()?.path, path.canonicalize()?);
    }
    session.command("recent")?;
    session.command(&format!("e {}", second.display()))?;
    assert_eq!(session.tabs.len(), 2);
    assert!(session.tabs[0].panes[0].is_recent());
    session.open(&first, true, session.screen.unwrap())?;
    let target = session.focused().id;
    session.activate(0)?;
    let source = session.focused().id;
    keys(&mut session, "<C-^>")?;
    assert_eq!(session.focused().id, target);
    assert_eq!(session.tabs[0].panes[0].id, source);
    assert_eq!(
      session.tabs[0].panes[0].alternate.as_ref().unwrap().path,
      first.canonicalize()?
    );
  }
  Ok(())
}

#[test]
fn split_file_aliases_and_reload() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("book.pdf");
  crate::fixtures::pdf(&path, &["one"], "")?;
  for native in [false, true] {
    for split in ["new", "vnew"] {
      let mut session = session(native, None)?;
      session.command(&format!("{split} {}", path.display()))?;
      assert_eq!(session.tab().panes.len(), 2);
      assert!(session.tab().panes[0].is_recent());
      assert_eq!(session.app()?.path, path.canonicalize()?);
      let generation = session.app()?.document_generation;
      session.command("e!")?;
      assert!(session.app()?.document_generation > generation);
      let id = session.focused().id;
      assert!(session.command("bd book.pdf").is_err());
      assert_eq!(session.focused().id, id);
    }
    let mut session = session(native, None)?;
    assert!(session.command("e!").is_err());
    assert!(session.focused().is_recent());
  }
  Ok(())
}

#[test]
fn recent_focus_and_gestures() -> Result<()> {
  use vpdf_model::scene::{Layer, Op, Recorder};
  use vpdf_ui::input::wheel::Precise;
  let mut session = session(false, None)?;
  session.command("new")?;
  for focus in [0, 1] {
    session.focus(0, focus)?;
    let area = session.area();
    let mut scene = Recorder::new(|_| false);
    Session::scene_tab(
      &mut session.tabs[0],
      area,
      None,
      &mut scene,
      |pane, scene| pane.scene_pane(scene, false),
    )?;
    assert_eq!(
      scene
        .into_scene()
        .iter()
        .filter(
          |op| matches!(op, Op::Fill(fill) if fill.layer() == Layer::AboveText)
        )
        .count(),
      1
    );
    let observed = session.observe();
    assert!(observed.complete);
    assert_eq!(observed.outcome.panel, "RECENT");
    assert_eq!(observed.outcome.pane, Some(session.focused().id));
    let pane = session.focused();
    let mouse = MouseEvent::new(
      MouseEventKind::ScrollDown,
      pane.at.column + 2,
      pane.at.row + 2,
    );
    for gesture in [
      native::Gesture::Pinch(1.1),
      native::Gesture::Turn(Precise { turns: 0.25, whole: 0 }),
    ] {
      assert!(!session.gesture_report(mouse.into(), gesture)?);
    }
  }
  Ok(())
}

#[test]
fn confirmation_holds_pane_focus() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("book.pdf");
  crate::fixtures::pdf(&path, &["one"], "")?;
  let mut session = session(false, None)?;
  session.command(&format!("e {}", path.display()))?;
  session.command("vnew")?;
  keys(&mut session, " x")?;
  assert!(session.confirming_removal());
  let focused = session.focused().id;
  let doc =
    session.tabs[0].panes.iter().find(|pane| !pane.is_recent()).unwrap();
  let mouse = MouseEvent::new(
    MouseEventKind::Down(MouseButton::Left),
    doc.at.column + 2,
    doc.at.row + 2,
  );
  session.interact(interaction::Interaction::Mouse(mouse.into()))?;
  assert_eq!(session.focused().id, focused);
  assert!(session.confirming_removal());
  keys(&mut session, "<Esc>")?;
  assert!(!session.confirming_removal());
  assert!(path.exists());
  Ok(())
}

#[test]
fn changed_password_clears_alternate() -> Result<()> {
  for native in [false, true] {
    let root = tempfile::tempdir()?;
    let path = root.path().join("book.pdf");
    crate::fixtures::pdf(&path, &["one"], "")?;
    let mut source = vpdf_pdf::document::Document::open(&path, "")?;
    let encrypted = crate::encryption::encrypt(&mut source.native)?;
    let mut session = session(native, None)?;
    session.command(&format!("e {}", path.display()))?;
    session.command("recent")?;
    let pane = session.focused().id;
    std::fs::copy(encrypted.path(), &path)?;
    keys(&mut session, "<C-6>")?;
    assert_eq!(session.focused().id, pane);
    assert!(session.focused().is_recent());
    assert!(session.focused().alternate.is_none());
    assert!(session.unlock.is_none());
    assert!(session.recent().unwrap().view.error().is_some());
  }
  Ok(())
}

#[test]
fn recent_tabs_beat_divider() -> Result<()> {
  for native in [false, true] {
    let mut session = session(native, None)?;
    session.command("tabnew")?;
    session.activate(0)?;
    session.command("new")?;
    let pane = session.focused();
    let row = pane.screen.rows - 1;
    let column = (0..pane.screen.columns)
      .find(|&column| {
        matches!(
          pane.chrome_at(column, row),
          Some(vpdf_ui::chrome::Item::Tab(1))
        )
      })
      .expect("second tab label");
    let mouse = MouseEvent::new(
      MouseEventKind::Down(MouseButton::Left),
      pane.at.column + column,
      pane.at.row + row,
    );
    assert!(session.edge_at(mouse.column, mouse.row).is_none());
    session.interact(interaction::Interaction::Mouse(mouse.into()))?;
    assert_eq!(session.active, 1);
    assert_eq!(session.tabs[0].panes.len(), 2);
    assert!(session.resizing.is_none());
  }
  Ok(())
}

#[test]
fn delete_replaces_only_source() -> Result<()> {
  let root = tempfile::tempdir()?;
  let first = root.path().join("first.pdf");
  let second = root.path().join("second.pdf");
  crate::fixtures::pdf(&first, &["first", "second"], "")?;
  crate::fixtures::pdf(&second, &["other"], "")?;
  for native in [false, true] {
    let mut session = session(native, None)?;
    command(&mut session, &format!("e {}", first.display()));
    session.command("vsplit")?;
    command(&mut session, "2");
    let position = session.app()?.navigation_position();
    command(&mut session, &format!("e {}", second.display()));
    let peer = session.tab().panes[0].id;
    let source = session.tab().focus;
    command(&mut session, "bd");
    assert_eq!(session.tab().focus, source);
    assert_eq!(session.tab().panes[0].id, peer);
    assert_eq!(session.app()?.path, first.canonicalize()?);
    assert_eq!(session.app()?.navigation_position(), position);
    assert!(session.focused().alternate.is_none());
    assert_eq!(session.panes().count(), 2);
    assert_eq!(session.tab().panes.len(), 2);
  }
  Ok(())
}
