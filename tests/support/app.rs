pub use crate::common::events as raw;
use std::path::{Path, PathBuf};
use vpdf::Result;
use vpdf::app::App;
use vpdf::session::Session;
use vpdf::solo::Solo;
use vpdf::trace::Drawable;
use vpdf_model::input::{
  Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind,
};
use vpdf_ui::keys::{Action, Input};
use vpdf_ui::layout::Screen;
use vpdf_ui::pointer::{Hit, Region};
use vpdf_ui::prompt::History;

pub fn screen() -> Screen {
  crate::common::cells(121, 40)
}

pub fn corpus(name: &str) -> Result<PathBuf> {
  Ok(PathBuf::from(std::env::var("VPDF_CORPUS")?).join(name))
}

pub fn session(path: &Path, screen: Screen) -> Result<Session> {
  let mut session = Session::new(None);
  session.open(path, true, screen)?;
  Ok(session)
}

pub fn solo(name: &str, text_mode: bool, screen: Screen) -> Result<Solo> {
  Solo::open(&corpus(name)?, "", text_mode, screen)
}

pub fn action(app: &mut App, action: Action) -> Result<()> {
  app.action(Input::plain(action))
}

pub fn dispatch(session: &mut Session, event: &Event) -> Result<()> {
  match event {
    Event::Key(key) => session.app().key(*key, &mut History::default()),
    Event::Mouse(mouse) => session.mouse_report(*mouse).map(|_| ()),
    Event::Paste(_) => panic!("input bytes"),
  }
}

pub fn press(session: &mut Session, code: KeyCode) -> Result<()> {
  dispatch(session, &Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

pub fn mouse(
  session: &mut Session,
  kind: MouseEventKind,
  (column, row): (u16, u16),
) -> Result<bool> {
  session.mouse(MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE })
}

pub fn draw(app: &mut impl Drawable) -> Result<String> {
  let mut out = Vec::new();
  app.draw_frame(&mut out)?;
  Ok(String::from_utf8_lossy(&out).into_owned())
}

pub fn pane(session: &mut Session, index: usize) -> &mut App {
  &mut session.tab().panes[index]
}

pub fn cell_in(
  session: &mut Session,
  index: usize,
  want: Hit,
) -> Result<(u16, u16)> {
  let region = match want {
    Hit::Row(_) | Hit::Label(_) => Region::Panel,
    Hit::Thumbnail(_) => Region::Overview,
    _ => Region::Page,
  };
  let app = pane(session, index);
  let (columns, rows) = (app.screen.columns, app.screen.rows - 1);
  for row in 0..rows {
    for column in 0..columns {
      if app.hovered(region, column, row, None)? == want {
        return Ok((app.at.column + column, app.at.row + row));
      }
    }
  }
  anyhow::bail!("no cell hits {want:?}")
}
