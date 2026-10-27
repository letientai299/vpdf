//! Worker-owned native commands and presentation state.
use super::{Session, open_input, report};
use std::path::PathBuf;
use std::sync::Arc;
use vpdf_model::geometry::Screen;
use vpdf_model::text::Position;
use vpdf_model::tile::TileOwner;
use vpdf_ui::input::InputOwner;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Target(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestId(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DocumentId {
  pub pane: TileOwner,
  pub generation: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Focus {
  #[default]
  Home,
  HomePrompt,
  Reader(InputOwner),
  Password,
  Dialog,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
  Open,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Command {
  pub target: Target,
  pub action: Action,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effect {
  Open { request: RequestId },
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct Snapshot {
  pub target: Target,
  pub document: Option<DocumentId>,
  pub path: Option<PathBuf>,
  pub focus: Focus,
  pub commands: Vec<Action>,
  pub location: Option<Position>,
  pub selection: Option<(Position, Position)>,
  pub busy: bool,
  pub searching: bool,
}

impl Snapshot {
  pub fn command(&self, action: Action) -> Option<Command> {
    self
      .commands
      .contains(&action)
      .then_some(Command { target: self.target, action })
  }
}

#[derive(Default)]
pub(crate) struct State {
  pub snapshot: Arc<Snapshot>,
  pub effects: Vec<Effect>,
  pending: Option<(RequestId, Target)>,
  next: u64,
}

pub(super) fn refresh(session: &mut Session) {
  let app = session.tabs.get(session.active).map(|tab| &tab.panes[tab.focus]);
  let document = app.map(|app| DocumentId {
    pane: TileOwner::new(app.id),
    generation: app.document_generation,
  });
  let focus = if session.unlock.is_some() {
    Focus::Password
  } else if session.picker.is_some() || session.confirming_removal() {
    Focus::Dialog
  } else if session.home.as_ref().is_some_and(|home| home.view.typing()) {
    Focus::HomePrompt
  } else {
    app.map_or(Focus::Home, |app| {
      Focus::Reader(
        app
          .overlays
          .owner(app.panel.as_ref().is_some_and(|p| p.view.focused())),
      )
    })
  };
  let old = &session.native.snapshot;
  let target = if old.document == document && old.focus == focus {
    old.target
  } else {
    Target(old.target.0.checked_add(1).expect("Native target exhausted"))
  };
  let open = !session.quit
    && !session.blocked()
    && session.native.pending.is_none()
    && matches!(focus, Focus::Home | Focus::Reader(InputOwner::Content));
  let snapshot = Snapshot {
    target,
    document,
    path: app.map(|app| app.document_identity().to_owned()),
    focus,
    commands: if open { vec![Action::Open] } else { Vec::new() },
    location: app.map(crate::app::App::navigation_position),
    selection: app.and_then(|app| {
      app
        .reader
        .selection
        .map(|selection| (selection.anchor, app.reader.cursor))
    }),
    busy: app.is_some_and(crate::app::App::busy),
    searching: app.is_some_and(|app| app.search.phase().running()),
  };
  if **old != snapshot {
    session.native.snapshot = Arc::new(snapshot);
  }
}

pub(super) fn dispatch(session: &mut Session, command: Command) -> bool {
  if session.native.snapshot.command(command.action) != Some(command) {
    return false;
  }
  match command.action {
    Action::Open => {
      session.native.next =
        session.native.next.checked_add(1).expect("Native request exhausted");
      let request = RequestId(session.native.next);
      session.native.pending = Some((request, command.target));
      session.native.effects.push(Effect::Open { request });
    }
  }
  true
}

pub(super) fn picked(
  session: &mut Session,
  request: RequestId,
  paths: Vec<PathBuf>,
  screen: Screen,
) -> bool {
  let Some((pending, target)) = session.native.pending else { return false };
  if pending != request {
    return false;
  }
  session.native.pending = None;
  if target != session.native.snapshot.target
    || session.blocked()
    || session.quit
  {
    return true;
  }
  for path in paths {
    let result = open_input(session, path.into(), screen).map(|_| ());
    let failed = result.is_err();
    report(session, result);
    if failed {
      break;
    }
  }
  true
}
