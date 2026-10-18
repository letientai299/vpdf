use crate::Result;
use crate::app::{App, InputOwner, Request, TabMove, Window};
use crate::cmdline;
use crate::document::{Document, Target};
use crate::keys::{Action, Grammar, Input, PanelKind};
use crate::prompt::{History, Prompt, PromptEvent, PromptKind};
use crate::reader::{self, Mode};
use crate::render_worker::RenderWorker;
use crate::split::Axis;
use crate::text::{PageId, Position};
use crossterm::event::{KeyCode, KeyEvent};
use std::process::{Command, Stdio};

/// Commands the recent view takes: opening, windows, tabs, quitting.
const COVERED: [&str; 23] = [
  "edit",
  "tabedit",
  "tabnew",
  "split",
  "vsplit",
  "close",
  "only",
  "tabnext",
  "tabprevious",
  "tabclose",
  "recent",
  "toc",
  "overview",
  "text",
  "info",
  "help",
  "marks",
  "jumps",
  "annotations",
  "cancel",
  "fzf",
  "quit",
  "qall",
];

impl App {
  pub fn action(&mut self, input: Input) -> Result<()> {
    if self.region_input(input)? {
      return Ok(());
    }
    if input.action == Action::LocalFocus {
      if self.panel.is_none() {
        return Ok(());
      }
      self.blur()?;
      if let Some(panel) = &mut self.panel {
        panel.focused = !panel.focused;
        self.relayout()?;
      }
      return Ok(());
    }
    if input.action == Action::Cancel {
      return self.cancel_input();
    }
    if input.action == Action::YankPath {
      return self.yank_path(input.register);
    }
    if !self.panel_focused()
      && self.overview.is_some()
      && !matches!(input.action, Action::Panel(_) | Action::ClosePanel)
      && self.overview_input(input)?
    {
      return Ok(());
    }
    // `q` leaves text preview first and keeps any side panel open.
    if matches!(input.action, Action::Quit)
      && self.preview.is_some()
      && !self.panel_focused()
    {
      return self.close_text_preview();
    }
    if self.panel_input(input)? {
      return Ok(());
    }
    match input.action {
      Action::Panel(kind) => self.open_panel(kind)?,
      Action::ClosePanel => self.close_panel()?,
      Action::Search(backward) => self.begin_search(backward)?,
      Action::WordSearch => self.word_search()?,
      Action::Result(reverse) => self.next_result(reverse, input.count)?,
      Action::Follow => self.follow_cursor()?,
      Action::Command => self.prompt = Some(Prompt::new(PromptKind::Command)),
      Action::Mark(name) => self.set_mark(name)?,
      Action::Hints(copy) => self.start_hints(copy)?,
      Action::Window(command) => {
        self.request = Some(Request::Window {
          command,
          count: input.count,
          explicit: input.explicit,
          file: None,
        });
      }
      Action::Peek => self.preview_link()?,
      Action::Overview => self.open_overview()?,
      Action::Figure(forward) => self.move_figure(forward, input.count)?,
      Action::FigureFit => self.fit_figure()?,
      Action::Region => self.open_region()?,
      Action::GoMark(name, exact) => self.go_mark(name, exact)?,
      Action::Tab(forward) => {
        self.request = Some(Request::Tab(match (forward, input.explicit) {
          (true, true) => TabMove::Go(input.count),
          (true, false) => TabMove::Next(1),
          (false, _) => TabMove::Previous(input.count),
        }));
      }
      _ => {
        if !self.navigate(input)? {
          self.viewport(input)?;
        }
      }
    }
    Ok(())
  }

  pub fn key(&mut self, key: KeyEvent, history: &mut History) -> Result<()> {
    let key = crate::keys::normalize(key);
    // Keys end pointer gestures.
    self.drop_sweep();
    self.drop_hover_preview();
    if self.prompt.is_some() {
      return self.edit_prompt(history, |prompt, history| {
        cmdline::feed(prompt, key, history)
      });
    }
    if key.code == KeyCode::Esc {
      return self.cancel_input();
    }
    if let Some(hints) = &self.hints {
      let picked = key.code == KeyCode::Enter && hints.selected.is_some();
      if crate::keys::plain(key)
        && matches!(key.code, KeyCode::Char(ch) if ch.is_ascii_alphabetic())
        || key.code == KeyCode::Backspace
        || picked
      {
        return self.hint_key(key);
      }
      self.hints = None;
    }
    let owner = self.input_owner();
    if matches!(
      owner,
      InputOwner::Context | InputOwner::Peek | InputOwner::Region
    ) {
      let input = self.keys.feed(key, Grammar::Other);
      if self.keys.awaiting() == Some("C-w") {
        self.dismiss_overlay();
      }
      let Some(input) = input else { return Ok(()) };
      return match owner {
        InputOwner::Context => self.context_input(input),
        InputOwner::Peek => self.preview_input(input),
        _ => self.action(input),
      };
    }
    let grammar = match self.input_owner() {
      InputOwner::Content
        if matches!(self.reader.mode(), Mode::Visual(_))
          || self.reader.operator.is_some() =>
      {
        Grammar::Objects
      }
      InputOwner::Content if self.reader.mode() == Mode::Normal => {
        Grammar::Normal
      }
      InputOwner::Content => Grammar::Cursor,
      _ => Grammar::Other,
    };
    if let Some(input) = self.keys.feed(key, grammar) {
      self.action(input)?;
    }
    Ok(())
  }

  pub(crate) fn cancel_input(&mut self) -> Result<()> {
    self.animation = None;
    let owner = self.input_owner();
    let pending =
      !self.keys.pending().is_empty() || self.reader.operator.is_some();
    if matches!(
      owner,
      InputOwner::Peek | InputOwner::Region | InputOwner::Context
    ) {
      if !pending {
        self.dismiss_overlay();
      }
      self.disarm();
      return Ok(());
    }
    if owner == InputOwner::Prompt
      && (self.peek.is_some()
        || self.region.as_ref().is_some_and(crate::region::Region::is_bar))
    {
      self.cancel_prompt()?;
      self.disarm();
      return Ok(());
    }
    self.region = None;
    self.drop_sweep();
    match owner {
      InputOwner::Prompt => self.cancel_prompt()?,
      InputOwner::Hints => self.hints = None,
      _ if self.overview.is_some() => self.close_overview(),
      _ => {}
    }
    if owner == InputOwner::Content && pending {
      self.reader.message.clear();
    } else {
      if owner == InputOwner::Content && self.reader.mode() == Mode::Cursor {
        self.reader.normal();
      }
      self.reader.cancel();
    }
    self.disarm();
    Ok(())
  }

  /// Pasted text goes to an open prompt. In Normal mode a pasted PDF
  /// path opens, since terminals without drops paste dropped paths.
  pub fn paste(&mut self, text: &str, history: &mut History) -> Result<()> {
    if self.prompt.is_some() {
      return self.edit_prompt(history, |prompt, _| Ok(prompt.paste(text)));
    }
    if self.reader.mode() == Mode::Normal
      && let Some(path) = crate::dnd::pdf(text)
    {
      self.request = Some(Request::Open { path, tab: false });
    }
    Ok(())
  }

  /// Applies `edit` to the open prompt and acts on its event.
  fn edit_prompt(
    &mut self,
    history: &mut History,
    edit: impl FnOnce(&mut Prompt, &History) -> Result<PromptEvent>,
  ) -> Result<()> {
    let Some(prompt) = &mut self.prompt else { return Ok(()) };
    let kind = prompt.kind;
    match edit(prompt, history)? {
      PromptEvent::Edited if matches!(kind, PromptKind::Search(_)) => {
        let query = prompt.text.clone();
        self.incremental(&query)?;
      }
      PromptEvent::Pending
      | PromptEvent::Edited
      | PromptEvent::Editor
      | PromptEvent::Complete => {}
      PromptEvent::Cancel => self.cancel_input()?,
      PromptEvent::Submit => {
        let Some(prompt) = self.prompt.take() else { return Ok(()) };
        history.push(prompt.kind, &prompt.text);
        match prompt.kind {
          PromptKind::Search(backward) => {
            self.finish_search(&prompt.text, backward)?;
          }
          PromptKind::Command => self.command(&prompt.text)?,
          PromptKind::Password => self.reload_with(&prompt.text)?,
        }
      }
    }
    Ok(())
  }

  /// Closes the prompt; searches return.
  pub fn cancel_prompt(&mut self) -> Result<()> {
    let Some(prompt) = self.prompt.take() else { return Ok(()) };
    if matches!(prompt.kind, PromptKind::Search(_)) {
      self.abort_search()?;
    }
    Ok(())
  }

  pub fn command(&mut self, line: &str) -> Result<()> {
    let (name, argument) = cmdline::parse(line);
    if name.is_empty() {
      return Ok(());
    }
    let spec = cmdline::lookup(name);
    // The covered document counts as closed, as on the start screen.
    if self.covered() {
      let kept = spec.is_some_and(|spec| COVERED.contains(&spec.name));
      anyhow::ensure!(kept, "Open a document before :{name}");
    }
    // Labels like `iv` beat prefixes like `:info`, not exact names.
    let label =
      cmdline::exact(name).is_none() && self.doc.page_number(name).is_some();
    if label || name.chars().all(|ch| ch.is_ascii_digit()) {
      return self.page(name);
    }
    let spec =
      spec.ok_or_else(|| anyhow::anyhow!("Unknown command: {name}"))?;
    match spec.name {
      "split" | "vsplit" => {
        let axis =
          if spec.name == "split" { Axis::Rows } else { Axis::Columns };
        let file = (!argument.is_empty()).then(|| cmdline::expand(argument));
        self.request = Some(Request::Window {
          command: Window::Split(axis),
          count: 1,
          explicit: false,
          file,
        });
      }
      "close" | "only" => {
        let command =
          if spec.name == "close" { Window::Close } else { Window::Only };
        self.request = Some(Request::Window {
          command,
          count: 1,
          explicit: false,
          file: None,
        });
      }
      "tabnew" if argument.is_empty() => {
        self.request = Some(Request::RecentTab);
      }
      "edit" | "tabedit" | "tabnew" => {
        anyhow::ensure!(!argument.is_empty(), "File required");
        let path = cmdline::expand(argument);
        self.request = Some(Request::Open { path, tab: spec.name != "edit" });
      }
      "tabnext" => {
        let step = argument.parse().map_or(TabMove::Next(1), TabMove::Go);
        self.request = Some(Request::Tab(step));
      }
      "tabprevious" => self.request = Some(Request::Tab(TabMove::Previous(1))),
      "qall" => self.request = Some(Request::QuitAll),
      "toc" => self.open_panel(PanelKind::Outline)?,
      "overview" => self.open_overview()?,
      "crop" => self.toggle_crop()?,
      "recent" => self.open_panel(PanelKind::Recent)?,
      "marks" => self.open_panel(PanelKind::Marks)?,
      "jumps" => self.open_panel(PanelKind::Jumps)?,
      "annotations" => self.open_panel(PanelKind::Annotations)?,
      "fzf" => self.request = Some(Request::Pick),
      "cancel" => {
        self.search.cancel();
        if let Some(job) = self.ocr.take() {
          job.cancel();
        }
        self.cancel_outline();
        self.region_job = None;
        self.reader.message = "Jobs canceled".into();
      }
      "nohlsearch" => self.search.hidden = true,
      "cursorline" => self.cursorline = !self.cursorline,
      "text" => self.open_panel(PanelKind::Text)?,
      "info" => self.open_panel(PanelKind::Info)?,
      "reload" => self.reload_with(&self.password.clone())?,
      "password" => self.prompt = Some(Prompt::new(PromptKind::Password)),
      "ocr" => {
        self.start_ocr(if argument.is_empty() { "eng" } else { argument })?;
      }
      "help" => self.open_panel(PanelKind::Help)?,
      "quit" => self.quit = true,
      "tabclose" => self.request = Some(Request::TabClose),
      name => anyhow::bail!("Unhandled command: {name}"),
    }
    Ok(())
  }

  fn yank_path(&mut self, register: char) -> Result<()> {
    let register =
      self.reader.operator.take().map_or(register, |op| op.register);
    self.reader.store(register, self.path.display().to_string())?;
    self.reader.message = "Yanked file path".into();
    Ok(())
  }

  fn page(&mut self, label: &str) -> Result<()> {
    let page = self
      .doc
      .page_number(label)
      .ok_or_else(|| anyhow::anyhow!("Page label not found: {label}"))?;
    self.go_to(Position { page, ..Position::default() }, true)
  }

  pub fn reload_with(&mut self, password: &str) -> Result<()> {
    // A write during the open shows up as a later change.
    let stamp = crate::watch::stamp(&self.path);
    self.doc = Document::open(&self.path, password)?;
    self.layout.reflow(&self.doc.pages);
    self.stamp = stamp;
    password.clone_into(&mut self.password);
    self.worker = RenderWorker::new(&self.path, password, self.doc.lists());
    let previous = std::mem::take(&mut self.search);
    let rerun = !previous.hidden && !previous.query.text.is_empty();
    // An open outline panel waits on the new outline now.
    self.outline_load = crate::outline::Loading::Deferred;
    if self.panel.as_ref().is_some_and(|panel| panel.kind == PanelKind::Outline)
    {
      self.load_outline();
    }
    self.graphics.forget(self.id);
    if let Some(ocr) = self.ocr.take() {
      ocr.cancel();
    }
    self.reader.cancel();
    self.reader.clear_history();
    self.requested.clear();
    self.reader.cursor = reader::clamp(self.reader.cursor, &mut self.doc)?;
    self.text_top = self.clamp_text(self.text_top)?;
    self.text_drawn = None;
    self.target = None;
    self.landing = None;
    self.point(None);
    self.link_press = crate::links::LinkPress::Idle;
    self.dismiss_overlay();
    if let Some(mut origin) = self.search_origin {
      origin.cursor = reader::clamp(origin.cursor, &mut self.doc)?;
      origin.anchor =
        origin.anchor.filter(|(page, _)| page.0 < self.doc.pages.len());
      if let Some(top) = origin.text_top {
        origin.text_top = Some(self.clamp_text(top)?);
      }
      origin.target = None;
      self.search_origin = Some(origin);
    }
    if let Some(panel) = &self.panel {
      let kind = panel.kind;
      let rows = self.panel_rows(kind)?;
      // These rows come from the session or refill when work finishes.
      let refills = match kind {
        PanelKind::Recent | PanelKind::Marks => true,
        PanelKind::Results => rerun,
        PanelKind::Outline => self.outline_pending(),
        _ => false,
      };
      let note = self.panel_note(kind);
      if let Some(panel) = self.panel.as_mut().filter(|_| !rows.is_empty()) {
        panel.refill(rows, note);
      } else if !refills {
        self.panel = None;
      }
    }
    if rerun {
      self.rerun(&previous.query)?;
    }
    self.relayout()?;
    self.reader.message = "Reloaded".into();
    Ok(())
  }

  /// Enter follows a link under the cursor and ignores plain text.
  pub fn follow_cursor(&mut self) -> Result<()> {
    let Some(position) = self.explicit_target()? else { return Ok(()) };
    match self.link_target(position)? {
      Some(target) => self.follow(&target),
      None => Ok(()),
    }
  }

  /// Destination of the link covering the cursor glyph.
  pub fn link_target(&mut self, cursor: Position) -> Result<Option<Target>> {
    let Some((page, x, y)) = self.glyph_point(cursor)? else { return Ok(None) };
    self.link_on(page, x, y)
  }

  /// Link destination at a page point.
  fn link_on(
    &mut self,
    page: PageId,
    x: f32,
    y: f32,
  ) -> Result<Option<Target>> {
    let Some(index) = self.link_at(page, x, y)? else { return Ok(None) };
    Ok(self.link(page, index)?.map(|link| link.target))
  }

  /// A page's link by index.
  pub(crate) fn link(
    &mut self,
    page: PageId,
    index: usize,
  ) -> Result<Option<crate::document::Link>> {
    Ok(self.doc.links(page)?.get(index).cloned())
  }

  /// Centre of the glyph at `position`.
  pub(crate) fn glyph_point(
    &mut self,
    position: Position,
  ) -> Result<Option<(PageId, f32, f32)>> {
    let text = self.doc.text(position.page)?;
    Ok(text.glyph(position).map(|glyph| {
      let (x, y) = glyph.bounds.centre();
      (position.page, x, y)
    }))
  }

  /// Index of the link covering a page point.
  pub(crate) fn link_at(
    &mut self,
    page: PageId,
    x: f32,
    y: f32,
  ) -> Result<Option<usize>> {
    let links = self.doc.links(page)?;
    Ok(links.iter().position(|link| link.bounds.contains(x, y)))
  }

  /// The glyph a page target lands on.
  fn landing(
    &mut self,
    page: PageId,
    point: Option<(f32, f32)>,
  ) -> Result<Position> {
    let page = PageId(page.0.min(self.doc.pages.len() - 1));
    Ok(match point {
      Some((x, y)) => self.doc.text(page)?.nearest(page, x, y),
      None => Position { page, ..Position::default() },
    })
  }

  pub fn follow(&mut self, target: &Target) -> Result<()> {
    let uri = match target {
      Target::Page(page) => return self.go_landing(target, *page, None),
      Target::Point { page, x, y } => {
        return self.go_landing(target, *page, Some((*x, *y)));
      }
      Target::Uri(uri) => uri,
    };
    let Some(uri) = openable(uri) else {
      anyhow::bail!("Unsupported link scheme");
    };
    let executable =
      if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    let mut child = Command::new(executable)
      .arg(uri)
      .stdin(Stdio::null())
      .stdout(Stdio::null())
      .stderr(Stdio::null())
      .spawn()?;
    std::thread::spawn(move || {
      let _ = child.wait();
    });
    Ok(())
  }

  /// Goes to a page target, describing the jump.
  fn go_landing(
    &mut self,
    target: &Target,
    page: PageId,
    point: Option<(f32, f32)>,
  ) -> Result<()> {
    let position = self.landing(page, point)?;
    let description = self.link_description(target, position);
    // Points land where they aim.
    if point.is_some() {
      self.land_at(position)?;
    } else {
      self.go_to(position, true)?;
    }
    self.reader.describe_jump(Some(description));
    Ok(())
  }

  /// Splits at an internal target; copies addresses.
  pub(crate) fn open_split(&mut self, target: &Target) -> Result<()> {
    let position = match target {
      Target::Uri(uri) => return self.copy_address(uri.clone()),
      Target::Page(page) => self.landing(*page, None)?,
      Target::Point { page, x, y } => self.landing(*page, Some((*x, *y)))?,
    };
    self.request = Some(Request::SplitAt(position));
    Ok(())
  }

  pub(crate) fn copy_address(&mut self, uri: String) -> Result<()> {
    self.reader.store('"', uri)?;
    self.reader.message = "Copied link".into();
    Ok(())
  }
}

/// What the system opener gets for `uri`: web addresses whole, mail
/// addresses without the query, whose `attach=` some clients obey.
fn openable(uri: &str) -> Option<&str> {
  if uri.starts_with("https://") || uri.starts_with("http://") {
    return Some(uri);
  }
  uri.starts_with("mailto:").then(|| uri.split('?').next().unwrap_or(uri))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn only_web_and_mail_open() {
    let cases = [
      ("https://a.org/?q=1", Some("https://a.org/?q=1")),
      ("http://a.org", Some("http://a.org")),
      ("mailto:me@a.org", Some("mailto:me@a.org")),
      ("mailto:me@a.org?attach=/etc/passwd", Some("mailto:me@a.org")),
      ("file:///etc/passwd", None),
      ("javascript:alert(1)", None),
      ("-a", None),
    ];
    for (uri, want) in cases {
      assert_eq!(openable(uri), want, "{uri}");
    }
  }
}
