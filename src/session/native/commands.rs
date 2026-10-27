//! Worker-owned native commands and presentation state.
use super::{Session, report};
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
  NewTab,
  CloseTab,
  NextTab,
  PreviousTab,
  Find,
  NextResult,
  PreviousResult,
  Copy,
  Cut,
  Paste,
  SelectAll,
  Undo,
  Print,
  Quit,
  FitWidth,
  FitPage,
  ZoomIn,
  ZoomOut,
  Rotate,
  Back,
  Forward,
  Outline,
  Overview,
  Sidebar,
  GoToPage,
  ClosePane,
  Reveal,
  Help,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Command {
  pub target: Target,
  pub action: Action,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Effect {
  Open {
    request: RequestId,
  },
  /// A successful user open for system recents.
  Recent(PathBuf),
  Reveal(PathBuf),
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct Snapshot {
  pub target: Target,
  pub document: Option<DocumentId>,
  pub path: Option<PathBuf>,
  pub focus: Focus,
  pub commands: Vec<Action>,
  pub location: Option<Position>,
  pub pages: usize,
  /// The active pane's panel.
  pub panel: Option<vpdf_ui::keys::PanelKind>,
  pub selection: Option<(Position, Position)>,
  pub busy: bool,
  pub searching: bool,
  pub all_selected: bool,
  pub edit_selection: Option<std::ops::Range<usize>>,
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
  pub(super) clipboard: Option<String>,
  all: Option<Target>,
  copy: Option<CopyJob>,
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
  let commands = available(session, focus, target);
  let snapshot = Snapshot {
    target,
    document,
    path: app.map(|app| app.document_identity().to_owned()),
    focus,
    commands,
    location: app.map(crate::app::App::navigation_position),
    pages: app.map_or(0, |app| app.doc.pages.len()),
    panel: app.and_then(|app| app.panel.as_ref()).map(|p| p.view.kind()),
    selection: app.and_then(|app| {
      app
        .reader
        .selection
        .map(|selection| (selection.anchor, app.reader.cursor))
    }),
    busy: app.is_some_and(crate::app::App::busy),
    searching: app.is_some_and(|app| app.search.phase().running()),
    all_selected: session.native.all == Some(target),
    edit_selection: prompt(session)
      .and_then(vpdf_ui::prompt::Prompt::selection),
  };
  let stale = session.native.all.is_some_and(|old| old != target);
  if **old != snapshot {
    session.native.snapshot = Arc::new(snapshot);
  }
  if stale {
    cancel_selection(session);
  }
}

fn available(session: &Session, focus: Focus, target: Target) -> Vec<Action> {
  let app = session.tabs.get(session.active).map(|tab| &tab.panes[tab.focus]);
  let open = !session.quit
    && !session.blocked()
    && session.native.pending.is_none()
    && matches!(focus, Focus::Home | Focus::Reader(InputOwner::Content));
  let editable = matches!(
    focus,
    Focus::HomePrompt | Focus::Password | Focus::Reader(InputOwner::Prompt)
  );
  let readable = matches!(focus, Focus::Reader(InputOwner::Content));
  let ordinary = !session.quit
    && !session.blocked()
    && session.native.pending.is_none()
    && !matches!(focus, Focus::Dialog);
  let mut commands = vec![Action::Quit];
  if open {
    commands.push(Action::Open);
  }
  if ordinary {
    commands.extend([Action::CloseTab, Action::NewTab, Action::Help]);
    if matches!(focus, Focus::Reader(_)) {
      commands.push(Action::Find);
    }
    if matches!(focus, Focus::Reader(InputOwner::Panel)) {
      commands.extend([Action::Copy, Action::Sidebar]);
    }
    if session.tabs.len() > 1 {
      commands.extend([Action::NextTab, Action::PreviousTab]);
    }
    if matches!(focus, Focus::Reader(InputOwner::Content | InputOwner::Prompt))
      && app.is_some_and(|app| !app.search.query().text.is_empty())
    {
      commands.extend([Action::NextResult, Action::PreviousResult]);
    }
    if readable && let Some(app) = app {
      reading(session, app, target, &mut commands);
    }
  }
  if editable && !session.quit {
    commands.extend([Action::SelectAll, Action::Paste]);
    if prompt(session).is_some_and(vpdf_ui::prompt::Prompt::can_undo) {
      commands.push(Action::Undo);
    }
    if prompt(session).and_then(vpdf_ui::prompt::Prompt::selected).is_some() {
      commands.extend([Action::Copy, Action::Cut]);
    }
  }

  commands
}

fn reading(
  session: &Session,
  app: &crate::app::App,
  target: Target,
  commands: &mut Vec<Action>,
) {
  commands.extend([
    Action::SelectAll,
    Action::FitWidth,
    Action::FitPage,
    Action::ZoomIn,
    Action::ZoomOut,
    Action::Rotate,
    Action::Back,
    Action::Forward,
    Action::Outline,
    Action::Overview,
    Action::Sidebar,
    Action::GoToPage,
    Action::ClosePane,
  ]);
  if app.manual.is_none() {
    commands.push(Action::Reveal);
  }
  if app.reader.selection.is_some() || session.native.all == Some(target) {
    commands.push(Action::Copy);
  }
  if cfg!(target_os = "macos")
    && session.printing.is_none()
    && app.manual.is_none()
  {
    commands.push(Action::Print);
  }
}

/// Whether `command` is `action` on the current target.
pub(super) fn accepts(
  session: &Session,
  command: Command,
  action: Action,
) -> bool {
  command.action == action
    && session.native.snapshot.command(action) == Some(command)
}

pub(super) fn dispatch(
  session: &mut Session,
  command: Command,
  screen: Screen,
) -> bool {
  if !accepts(session, command, command.action) {
    return false;
  }
  let result = execute(session, command, screen);
  report(session, result);
  true
}

fn execute(
  session: &mut Session,
  command: Command,
  screen: Screen,
) -> crate::Result<()> {
  use vpdf_ui::keys::{Action as Reader, Input};
  match command.action {
    Action::Open => {
      session.native.next =
        session.native.next.checked_add(1).expect("Native request exhausted");
      let request = RequestId(session.native.next);
      session.native.pending = Some((request, command.target));
      session.native.effects.push(Effect::Open { request });
    }
    Action::Quit => session.quit = true,
    Action::CloseTab => {
      if session.tabs.is_empty() {
        session.quit = true;
      } else {
        session.close()?;
        super::ensure_home(session, screen);
      }
    }
    Action::NewTab => {
      if session.tabs.is_empty() {
        super::ensure_home(session, screen);
      } else {
        session.recent_tab(screen)?;
      }
    }
    Action::NextTab | Action::PreviousTab => {
      let forward = command.action == Action::NextTab;
      let step = if forward {
        vpdf_ui::window::TabMove::Next(1)
      } else {
        vpdf_ui::window::TabMove::Previous(1)
      };
      session.activate(session.step(step))?;
    }
    Action::Copy | Action::Cut => {
      if let Some(text) =
        prompt(session).and_then(|p| p.selected().map(str::to_owned))
      {
        session.native.clipboard = Some(text);
        if command.action == Action::Cut {
          edit_key(session, 'x', screen);
        }
      } else if session.native.all == Some(command.target) {
        session.native.copy = Some(CopyJob {
          target: command.target,
          page: 0,
          text: String::new(),
        });
      } else {
        session.app().action(Input::plain(Reader::Yank))?;
      }
    }
    Action::SelectAll if prompt(session).is_none() => {
      session.app().reader.normal();
      session.app().reader.select_document(true);
      session.native.all = Some(command.target);
      session.app().reader.notice("All document text selected");
    }
    Action::SelectAll | Action::Undo => {
      let ch = if command.action == Action::Undo { 'z' } else { 'a' };
      edit_key(session, ch, screen);
    }
    // These carry values through their own inputs.
    Action::Paste | Action::GoToPage => {}
    Action::Print => session.print()?,
    Action::Reveal => {
      let path = session.app().path.clone();
      session.native.effects.push(Effect::Reveal(path));
    }
    Action::Help if session.tabs.is_empty() => {
      session.request_help("", vpdf_ui::topics::Invocation::home(), screen)?;
    }
    Action::Find => find(session, screen)?,
    action => reader_action(session, action, screen)?,
  }
  Ok(())
}

fn find(session: &mut Session, screen: Screen) -> crate::Result<()> {
  use vpdf_ui::prompt::PromptKind;
  if prompt(session)
    .is_some_and(|prompt| matches!(prompt.kind(), PromptKind::Search(_)))
  {
    edit_key(session, 'a', screen);
    return Ok(());
  }
  if prompt(session).is_some() {
    session.app().cancel_input()?;
  }
  if let Some(panel) = &mut session.app().panel {
    panel.view.focus(false);
  }
  let query = session.app().search.query().text.clone();
  session.app().begin_search(false)?;
  if !query.is_empty() {
    super::apply_reader(session, super::Input::Paste(query), &mut { screen });
    edit_key(session, 'a', screen);
  }
  Ok(())
}

fn reader_action(
  session: &mut Session,
  action: Action,
  screen: Screen,
) -> crate::Result<()> {
  use vpdf_ui::keys::{Action as Reader, Input};

  let native = action;
  let action = match action {
    Action::NextResult => Reader::Result(false),
    Action::PreviousResult => Reader::Result(true),
    Action::FitWidth => Reader::Fit,
    Action::FitPage => Reader::FitPage,
    Action::ZoomIn => Reader::Zoom(1),
    Action::ZoomOut => Reader::Zoom(-1),
    Action::Rotate => Reader::Rotate,
    Action::Back => Reader::History(false),
    Action::Forward => Reader::History(true),
    Action::Sidebar if session.app().panel.is_some() => Reader::ClosePanel,
    Action::Outline | Action::Sidebar => {
      Reader::Panel(vpdf_ui::keys::PanelKind::Outline)
    }
    Action::Overview => Reader::Overview,
    Action::ClosePane => Reader::Window(vpdf_ui::window::Window::Close),
    Action::Help => Reader::Help,
    _ => unreachable!(),
  };
  session.app().action(Input::plain(action))?;
  // The sidebar control keeps reading focus.
  if native == Action::Sidebar
    && let Some(panel) = &mut session.app().panel
  {
    panel.view.focus(false);
  }
  session.settle()?;
  super::ensure_home(session, screen);
  Ok(())
}

/// Goes to a one-based page.
pub(super) fn go_to_page(
  session: &mut Session,
  command: Command,
  page: usize,
) -> bool {
  if !accepts(session, command, Action::GoToPage) {
    return false;
  }
  let page = vpdf_model::text::PageId(page.saturating_sub(1));
  let result = session.app().go_to_page(page).and_then(|()| session.settle());
  report(session, result);
  true
}

fn edit_key(session: &mut Session, ch: char, mut screen: Screen) {
  use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};
  super::apply_reader(
    session,
    super::Input::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::SUPER)),
    &mut screen,
  );
}

pub(super) fn prepare_prompt(session: &mut Session) {
  let prompt = if let Some(unlock) = &mut session.unlock {
    Some(&mut unlock.prompt)
  } else if let Some(home) = &mut session.home {
    home.view.prompt_mut()
  } else {
    session
      .tabs
      .get_mut(session.active)
      .and_then(|tab| tab.panes[tab.focus].overlays.prompt_mut())
  };
  if let Some(prompt) = prompt {
    prompt.native();
  }
}

fn prompt(session: &Session) -> Option<&vpdf_ui::prompt::Prompt> {
  if let Some(unlock) = &session.unlock {
    return Some(&unlock.prompt);
  }
  if let Some(home) = &session.home {
    return home.view.prompt();
  }
  let tab = session.tabs.get(session.active)?;
  tab.panes[tab.focus].overlays.prompt()
}

struct CopyJob {
  target: Target,
  page: usize,
  text: String,
}

pub(super) fn copying(session: &Session) -> bool {
  session.native.copy.is_some()
}

pub(super) fn cancel_selection(session: &mut Session) {
  session.native.all = None;
  session.native.copy = None;
  for app in session.panes_mut() {
    app.reader.select_document(false);
  }
}

pub(super) fn copy_update(session: &mut Session) -> bool {
  let Some(mut job) = session.native.copy.take() else { return false };
  refresh(session);
  if session.native.snapshot.target != job.target
    || session.native.all != Some(job.target)
  {
    return true;
  }
  let result = (|| -> crate::Result<bool> {
    let app = session.app();
    let page = app.doc.text(vpdf_model::text::PageId(job.page))?;
    anyhow::ensure!(!page.truncated, "Page text exceeds extraction limit");
    for line in &page.lines {
      let text = line.text();
      anyhow::ensure!(
        job.text.len() + text.len() + 1 < vpdf_reader::reader::YANK_LIMIT,
        "Selection exceeds {} MiB",
        vpdf_reader::reader::YANK_LIMIT >> 20
      );
      job.text.push_str(&text);
      job.text.push('\n');
    }
    job.page += 1;
    Ok(job.page == app.doc.pages.len())
  })();
  match result {
    Ok(true) => session.native.clipboard = Some(job.text),
    Ok(false) => session.native.copy = Some(job),
    Err(error) => report(session, Err(error)),
  }
  true
}

pub(in crate::session) fn note_recent(
  session: &mut Session,
  path: &std::path::Path,
) {
  if matches!(session.frontend, crate::raster::Frontend::Native(_)) {
    session.native.effects.push(Effect::Recent(path.to_owned()));
  }
}

pub(super) fn picked(
  session: &mut Session,
  request: RequestId,
  paths: Vec<PathBuf>,
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
  // The run loop opens one per turn, so the first shows promptly.
  session.native_opens.extend(paths.into_iter().map(Into::into));
  true
}
