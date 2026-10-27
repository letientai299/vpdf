pub use crate::common::events as raw;
use anyhow::Context as _;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use vpdf::Result;
use vpdf::app::App;
use vpdf::session::Session;
use vpdf::solo::Solo;
use vpdf::trace::Drawable;
use vpdf_model::input::{Event, MouseButton, MouseEvent, MouseEventKind};
use vpdf_ui::keys::{Action, Input};
use vpdf_ui::layout::Screen;
use vpdf_ui::pointer::{Hit, Region, Route};

pub fn screen() -> Screen {
  crate::common::cells(121, 40)
}

pub fn corpus(name: &str) -> Result<PathBuf> {
  Ok(PathBuf::from(std::env::var("VPDF_CORPUS")?).join(name))
}

/// A temporary copy of corpus file `name`.
pub fn temp_copy(name: &str) -> Result<(TempDir, PathBuf)> {
  crate::common::temp_pdf(|path| {
    std::fs::copy(corpus(name)?, path)?;
    Ok(())
  })
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
    Event::Key(key) => session.type_key(*key),
    Event::Mouse(mouse) => session.mouse_report(*mouse).map(|_| ()),
    Event::Paste(_) => panic!("input bytes"),
  }
}

/// Dispatches the events decoded from `bytes`.
pub fn press(session: &mut Session, bytes: &[u8]) -> Result<()> {
  raw(bytes).iter().try_for_each(|event| dispatch(session, event))
}

pub fn mouse(
  session: &mut Session,
  kind: MouseEventKind,
  (column, row): (u16, u16),
) -> Result<bool> {
  session.mouse(MouseEvent::new(kind, column, row))
}

/// A left-button drag from `from` to `to`.
pub fn drag(
  session: &mut Session,
  from: (u16, u16),
  to: (u16, u16),
) -> Result<()> {
  mouse(session, MouseEventKind::Down(MouseButton::Left), from)?;
  mouse(session, MouseEventKind::Drag(MouseButton::Left), to)?;
  mouse(session, MouseEventKind::Up(MouseButton::Left), to)?;
  Ok(())
}

/// A one-click `kind` on pane `pane`'s page at a cell.
pub fn page_route(
  pane: u32,
  kind: MouseEventKind,
  (column, row): (u16, u16),
) -> Route {
  let mouse = MouseEvent::new(kind, column, row);
  Route {
    pane,
    region: Region::Page,
    focus: true,
    mouse,
    clicks: 1,
    pixel: None,
  }
}

pub fn draw(app: &mut impl Drawable) -> Result<String> {
  let mut out = Vec::new();
  app.draw_frame(&mut out)?;
  Ok(String::from_utf8_lossy(&out).into_owned())
}

pub fn pane(session: &mut Session, index: usize) -> &mut App {
  session.tab().panes[index].document_mut().unwrap()
}

/// The pane with `id`, in any tab.
pub fn pane_id(session: &mut Session, id: u32) -> &mut App {
  let mut panes = session.tabs.iter_mut().flat_map(|tab| &mut tab.panes);
  panes
    .find(|app| app.id == id)
    .and_then(vpdf::session::Pane::document_mut)
    .expect("a document with the id")
}

pub fn cell_in(
  session: &mut Session,
  index: usize,
  want: Hit,
) -> Result<(u16, u16)> {
  let region = match want {
    Hit::Row(_) | Hit::Label(_) | Hit::PanelControl(_) => Region::Panel,
    Hit::Thumbnail(_) => Region::Overview,
    _ => Region::Page,
  };
  let app = pane(session, index);
  let rows = 0..app.screen.rows - 1;
  let found = cell_where(app, region, rows, |hit| hit == want);
  found.with_context(|| format!("no cell hits {want:?}"))
}

/// The first `region` cell over `rows` that passes `want`.
pub fn cell_where(
  app: &mut App,
  region: Region,
  rows: std::ops::Range<u16>,
  want: impl Fn(Hit) -> bool,
) -> Result<(u16, u16)> {
  for row in rows {
    for column in 0..app.screen.columns {
      if want(app.hovered(region, column, row, None)?) {
        return Ok((app.at.column + column, app.at.row + row));
      }
    }
  }
  anyhow::bail!("no {region:?} cell matches")
}
