use std::path::PathBuf;
use vpdf::Result;
use vpdf::app::{Request, TabMove};
use vpdf::layout::Screen;
use vpdf::session::Session;

#[test]
#[ignore = "requires verified public PDF corpus"]
fn tabs_share_graphics_and_close_in_order() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut session = Session::new("", false);
  for name in ["book.pdf", "paper.pdf", "form.pdf"] {
    session.open(&root.join(name), true, screen)?;
  }
  assert_eq!((session.tabs.len(), session.active), (3, 2));
  session.open(&root.join("book.pdf"), true, screen)?;
  assert_eq!((session.tabs.len(), session.active), (3, 0));
  let names = &session.app().tabline.names;
  assert_eq!(names, &["book.pdf", "paper.pdf", "form.pdf"]);
  let steps = [
    (TabMove::Previous(1), 2),
    (TabMove::Next(2), 1),
    (TabMove::Go(9), 2),
    (TabMove::Go(1), 0),
  ];
  for (step, expected) in steps {
    session.app().request = Some(Request::Tab(step));
    session.settle()?;
    assert_eq!(session.active, expected, "{step:?}");
  }
  let id = session.app().id;
  assert_eq!(session.app().graphics.doc, id);
  session.app().request =
    Some(Request::Open { path: root.join("ocr.pdf"), tab: false });
  session.settle()?;
  assert_eq!(session.app().name(), "ocr.pdf");
  assert_ne!(session.app().graphics.doc, id);
  session.app().command("tabclose")?;
  session.settle()?;
  assert_eq!((session.tabs.len(), session.active), (2, 0));
  assert_eq!(session.app().name(), "paper.pdf");
  session.app().command("qa")?;
  session.settle()?;
  assert!(session.quit);
  Ok(())
}
