use crate::common;
use crate::focus::{screen, send};
use base64::Engine;
use std::path::{Path, PathBuf};
use vpdf::Result;
use vpdf::home::{Home, Outcome};
use vpdf::session::Session;
use vpdf_terminal::decode::Event;
use vpdf_ui::prompt::History;

fn pdf(directory: &Path, name: &str) -> Result<PathBuf> {
  let path = directory.join(name);
  common::text_pdf(&path, &[name])?;
  Ok(std::fs::canonicalize(path)?)
}

/// Kitty's events for a drop of `data` as `mime` on screen cell
/// `(x, y)`: what the reader answered, and how the drop ended.
fn drop_on(
  session: &mut Session,
  (x, y): (u16, u16),
  mime: &str,
  data: &str,
) -> (String, Result<()>) {
  let data = base64::engine::general_purpose::STANDARD.encode(data);
  let mut out = Vec::new();
  let mut ended = Ok(());
  for body in [
    format!("t=m:x={x}:y={y}:X=0:Y=0:o=3:m=0;image/png {mime} "),
    format!("t=M:x={x}:y={y}:X=0:Y=0:o=3:m=0;image/png {mime} "),
    format!("t=r:x=2:m=0;{data}"),
    "t=r:x=2".to_owned(),
  ] {
    ended = session.drop_event(&body, &mut out).and_then(|_| session.settle());
    if ended.is_err() {
      break;
    }
  }
  (String::from_utf8_lossy(&out).into_owned(), ended)
}

fn uri(path: &Path) -> String {
  format!("file://{}\r\n", path.display().to_string().replace(' ', "%20"))
}

/// Documents of the active tab's panes, and the focused one.
fn shown(session: &mut Session) -> (Vec<PathBuf>, usize) {
  let tab = session.tab();
  (
    tab.panes.iter().map(|app| app.document().unwrap().path.clone()).collect(),
    tab.focus,
  )
}

const FILES: &str = "text/uri-list";

/// A session split in two panes on `a`: the right pane's index and its
/// first column.
fn split(a: &Path) -> Result<(Session, usize, u16)> {
  let mut session = crate::harness::session(a, screen())?;
  send(&mut session, b"\x17v")?;
  assert_eq!(session.tab().panes.len(), 2);
  // Whichever pane got focus.
  let right = session
    .tab()
    .panes
    .iter()
    .position(|app| app.at.column > 0)
    .expect("a right pane");
  let column = session.tab().panes[right].at.column;
  Ok((session, right, column))
}

#[test]
fn drops_open_tabs() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let root = directory.path();
  let (a, b, c, d) = (
    pdf(root, "a.pdf")?,
    pdf(root, "b b.pdf")?,
    pdf(root, "c.pdf")?,
    pdf(root, "d.pdf")?,
  );
  let (mut session, _, column) = split(&a)?;
  let owners: Vec<_> = session.tab().panes.iter().map(|app| app.id).collect();
  let (on_right, on_left) = ((column + 5, 5), (5, 5));
  let (replies, ended) = drop_on(&mut session, on_right, FILES, &uri(&b));
  ended?;
  assert!(replies.contains("\x1b]72;t=m:o=1;text/uri-list\x1b\\"));
  assert!(replies.contains("\x1b]72;t=r:x=2\x1b\\"));
  assert!(replies.ends_with("\x1b]72;t=r:o=1\x1b\\"));
  assert_eq!(session.tabs.len(), 2);
  assert_eq!(session.active, 1);
  assert_eq!(shown(&mut session), (vec![b.clone()], 0));
  let original = &session.tabs[0];
  assert_eq!(
    original.panes.iter().map(|app| app.id).collect::<Vec<_>>(),
    owners
  );
  assert!(original.panes.iter().all(|app| app.document().unwrap().path == a));
  // Reusing the current view preserves pending keys.
  send(&mut session, b"2")?;
  drop_on(&mut session, on_left, FILES, &uri(&b)).1?;
  assert_eq!(session.tabs.len(), 2);
  assert_eq!(session.active, 1);
  assert_eq!(session.app().unwrap().keys.pending(), "2");
  drop_on(&mut session, on_right, FILES, &uri(&a)).1?;
  assert_eq!(session.tabs.len(), 2);
  assert_eq!(session.active, 0);
  // Divider drops also open tabs.
  drop_on(&mut session, (column - 1, 5), FILES, &uri(&c)).1?;
  assert_eq!(session.tabs.len(), 3);
  assert_eq!(shown(&mut session), (vec![c], 0));
  // Keyboard editing still replaces the focused pane.
  send(&mut session, format!(":e {}\r", d.display()).as_bytes())?;
  assert_eq!(session.tabs.len(), 3);
  assert_eq!(shown(&mut session), (vec![d], 0));
  Ok(())
}

#[test]
fn file_and_text_drops() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let root = directory.path();
  let (a, c) = (pdf(root, "a.pdf")?, pdf(root, "c.pdf")?);
  let (mut session, _, column) = split(&a)?;
  let on_right = (column + 5, 5);
  // Files open tabs despite prompts.
  send(&mut session, b":e")?;
  drop_on(&mut session, on_right, FILES, &uri(&c)).1?;
  assert_eq!(session.tabs.len(), 2);
  assert_eq!(shown(&mut session), (vec![c], 0));
  assert!(
    session.tabs[0].panes.iter().all(|app| app.document().unwrap().path == a)
  );
  let before = shown(&mut session);
  send(&mut session, b":")?;
  let typed = |session: &mut Session| {
    session
      .app()
      .unwrap()
      .overlays
      .prompt()
      .map(|prompt| prompt.text().to_owned())
  };
  assert_eq!(typed(&mut session), Some(String::new()));
  let (replies, ended) =
    drop_on(&mut session, on_right, "text/plain;charset=utf-8", " page");
  ended?;
  assert!(replies.contains("t=m:o=1;text/plain;charset=utf-8"));
  assert_eq!(typed(&mut session), Some(" page".into()));
  session.app().unwrap().cancel_prompt()?;
  // Text is refused without a prompt.
  let (replies, ended) = drop_on(&mut session, on_right, "text/plain", "x");
  ended?;
  assert!(replies.contains("\x1b]72;t=m:o=0\x1b\\"));
  // A drop with no PDF opens nothing and is cancelled.
  let notes = root.join("notes.txt");
  std::fs::write(&notes, "text")?;
  let (replies, ended) = drop_on(&mut session, on_right, FILES, &uri(&notes));
  assert!(ended.is_err());
  assert!(replies.ends_with("\x1b]72;t=r:o=0\x1b\\"));
  assert_eq!(shown(&mut session), before);
  // The terminal's error ends the drop.
  let mut out = Vec::new();
  session.drop_event("t=M:x=5:y=5:o=1;text/uri-list", &mut out)?;
  let failed = session.drop_event("t=R:x=1;EIO:read failed", &mut out);
  let error = failed.expect_err("a failed drop").to_string();
  assert_eq!(error, "Drop failed: EIO:read failed");
  assert!(String::from_utf8_lossy(&out).ends_with("\x1b]72;t=r:o=0\x1b\\"));
  Ok(())
}

#[test]
fn pasted_pdf_paths_open_in_normal_mode() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let root = directory.path();
  let (a, b) = (pdf(root, "a.pdf")?, pdf(root, "b b.pdf")?);
  let mut session = crate::harness::session(&a, screen())?;
  let mut history = History::default();
  let pasted = b.display().to_string();
  // (keys first, pasted text) -> document shown
  let cases = [
    (b"v".as_slice(), pasted.clone(), a.clone()),
    (b"\x1b[27u", "not a path".to_owned(), a),
    (b"", pasted.replace(' ', "\\ "), b.clone()),
  ];
  for (keys, text, want) in cases {
    send(&mut session, keys)?;
    session.app().unwrap().paste(&text, &mut history)?;
    session.settle()?;
    assert_eq!(session.app().unwrap().path, want, "{text}");
  }
  // The start screen opens one too.
  let mut home = Home::new(&[], screen(), &vpdf_pdf::jobs::Wake::none(), true);
  let opened = home.event(Event::Paste(pasted), &mut history)?;
  assert_eq!(opened, Outcome::Reference(b.into()));
  Ok(())
}
