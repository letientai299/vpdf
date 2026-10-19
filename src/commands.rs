use crate::Result;
use crate::app::{App, Request};
use crate::cmdline;
use crate::document::Target;
use crate::text::{PageId, Position};
use crossterm::event::KeyEvent;
use std::process::{Command, Stdio};
use vpdf_reader::reader::Mode;
use vpdf_ui::commands::{self, View};
use vpdf_ui::input::InputOwner;
use vpdf_ui::keys::{Action, Grammar, Input, PanelKind};
use vpdf_ui::prompt::{History, Prompt, PromptEvent, PromptKind};
use vpdf_ui::split::Axis;
use vpdf_ui::window::{TabMove, Window};

/// The note after copying an address.
pub(crate) const COPIED_LINK: &str = "Copied link";

impl App {
  pub fn action(&mut self, input: Input) -> Result<()> {
    self.cancel_definition();
    self.search.forget_yank();
    if self.reader.prepare(input.action) {
      self.disarm();
    }
    if self.region_input(input)? {
      return Ok(());
    }
    if input.action == Action::LocalFocus {
      if self.panel.is_none() {
        return Ok(());
      }
      self.blur()?;
      if let Some(panel) = &mut self.panel {
        panel.view.focus(!panel.view.focused());
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
      Action::Search(backward) => {
        self.reader.multiply_operator(input.count);
        self.begin_search(backward)?;
      }
      Action::WordSearch => self.word_search()?,
      Action::Result(reverse) => self.next_result(reverse, input.count)?,
      Action::Follow => self.follow_cursor()?,
      Action::Definition => self.definition()?,
      Action::OpenUrl => self.open_cursor_url()?,
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
    self.search.forget_yank();
    let key = vpdf_ui::keys::normalize(key);
    // Keys end pointer gestures.
    self.end_gestures();
    self.drop_hover_preview();
    if self.prompt.is_some() {
      let view = self.command_view();
      return self.edit_prompt(history, |prompt, history| {
        cmdline::feed(prompt, key, history, view)
      });
    }
    if vpdf_ui::keys::cancels(key) {
      return self.cancel_input();
    }
    if let Some(hints) = &self.hints {
      if hints.accepts(key) {
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
    let grammar =
      owner.grammar(self.reader.mode(), self.reader.operator.is_some());
    if let Some(input) = self.keys.feed(key, grammar) {
      self.action(input)?;
    } else if self.keys.pending().is_empty() {
      self.reader.disarm();
    }
    Ok(())
  }

  pub(crate) fn cancel_input(&mut self) -> Result<()> {
    self.cancel_definition();
    self.reader.cancel_scroll();
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
        || self.region.as_ref().is_some_and(vpdf_ui::region::Region::is_bar))
    {
      self.cancel_prompt()?;
      self.disarm();
      return Ok(());
    }
    self.region = None;
    self.end_gestures();
    if matches!(owner, InputOwner::Prompt | InputOwner::Hints)
      || self.overview.is_some()
    {
      match owner {
        InputOwner::Prompt => self.cancel_prompt()?,
        InputOwner::Hints => self.hints = None,
        _ => self.close_overview(),
      }
      self.disarm();
      return Ok(());
    }
    if owner == InputOwner::Content && pending {
      self.reader.notice("");
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
    let kind = prompt.kind();
    match edit(prompt, history)? {
      PromptEvent::Edited if matches!(kind, PromptKind::Search(_)) => {
        let query = prompt.text().to_owned();
        self.incremental(&query)?;
      }
      PromptEvent::Pending
      | PromptEvent::Edited
      | PromptEvent::Editor
      | PromptEvent::Complete => {}
      PromptEvent::Cancel => self.cancel_input()?,
      PromptEvent::Submit => {
        let Some(prompt) = self.prompt.take() else { return Ok(()) };
        history.push(prompt.kind(), prompt.text());
        match prompt.kind() {
          PromptKind::Search(backward) => {
            self.finish_search(prompt.text(), backward)?;
          }
          PromptKind::Command => self.command(prompt.text())?,
          PromptKind::Password => self.reload_with(prompt.text())?,
        }
      }
    }
    Ok(())
  }

  /// Closes the prompt; searches return.
  pub fn cancel_prompt(&mut self) -> Result<()> {
    let Some(prompt) = self.prompt.take() else { return Ok(()) };
    if matches!(prompt.kind(), PromptKind::Search(_)) {
      self.abort_search()?;
    }
    Ok(())
  }

  fn command_view(&self) -> View {
    if self.covered() { View::Recent } else { View::Document }
  }

  pub fn command(&mut self, line: &str) -> Result<()> {
    let (spec, argument) = match commands::resolve(
      line,
      self.command_view(),
      cmdline::on_path,
      |name| self.doc.page_number(name).is_some(),
    )? {
      commands::Resolution::Empty => return Ok(()),
      commands::Resolution::Page(name) => return self.page(name),
      commands::Resolution::Command { spec, argument } => (spec, argument),
    };
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
        self.cancel_search();
        if let Some(job) = self.ocr.take() {
          job.cancel();
        }
        self.cancel_outline();
        self.cancel_crop()?;
        self.region_job = None;
        self.reader.notice("Jobs canceled");
      }
      "nohlsearch" => self.search.set_hidden(true),
      "cursorline" => self.cursorline = !self.cursorline,
      "perf" => {
        self.request = Some(Request::Perf(crate::perf::switch(argument)?));
      }
      "mouse" => {
        let on = match argument {
          "" => !vpdf_terminal::terminal::mouse(),
          "on" => true,
          "off" => false,
          _ => anyhow::bail!("Usage: :mouse [on|off]"),
        };
        self.request = Some(Request::Mouse(on));
        let message =
          if on { "Mouse on" } else { "Mouse off; :mouse on restores" };
        self.reader.notice(message);
      }
      "text" => self.open_panel(PanelKind::Text)?,
      "info" => self.open_panel(PanelKind::Info)?,
      "reload" => self.reload_with(&self.password.clone())?,
      "password" => self.prompt = Some(Prompt::new(PromptKind::Password)),
      "ocr" => {
        self.start_ocr(if argument.is_empty() { "eng" } else { argument })?;
      }
      "help" => self.open_panel(PanelKind::Help)?,
      "quit" => self.quit = true,
      "tabclose" => self.request = Some(Request::TabClose(None)),
      name => anyhow::bail!("Unhandled command: {name}"),
    }
    Ok(())
  }

  fn yank_path(&mut self, register: char) -> Result<()> {
    let register =
      self.reader.take_operator().map_or(register, |op| op.register);
    self.reader.store(register, self.path.display().to_string())?;
    self.reader.notice("Yanked file path");
    Ok(())
  }

  fn page(&mut self, label: &str) -> Result<()> {
    let page = self
      .doc
      .page_number(label)
      .ok_or_else(|| anyhow::anyhow!("Page label not found: {label}"))?;
    self.go_to_page(page)
  }

  /// Enter follows a link under the cursor and ignores plain text.
  pub fn follow_cursor(&mut self) -> Result<()> {
    let Some(position) = self.explicit_target()? else { return Ok(()) };
    match self.link_target(position)? {
      Some(target) => self.follow(&target),
      None => Ok(()),
    }
  }

  fn open_cursor_url(&mut self) -> Result<()> {
    let Some(position) = self.explicit_target()? else { return Ok(()) };
    if let Some(target @ Target::Uri(_)) = self.link_target(position)? {
      self.follow(&target)?;
    }
    Ok(())
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
    Ok(links.iter().position(|link| link.contains(x, y)))
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
    let position = self.reading_view().destination(page, point)?;
    let description = self.link_description(target, position);
    // Points land where they aim.
    if point.is_some() {
      self.land_at(position)?;
      self.reader.aim(target.clone());
    } else {
      self.go_to_page(position.page)?;
    }
    self.reader.describe_jump(Some(description));
    Ok(())
  }

  /// Splits at an internal target; copies addresses.
  pub(crate) fn open_split(&mut self, target: &Target) -> Result<()> {
    if let Target::Uri(uri) = target {
      return self.copy_address(uri.clone());
    }
    self.request = Some(Request::SplitTo(target.clone()));
    Ok(())
  }

  pub(crate) fn copy_address(&mut self, uri: String) -> Result<()> {
    self.copy_text(uri, COPIED_LINK)
  }

  /// Copies into the `"` register, saying `message`.
  pub(crate) fn copy_text(
    &mut self,
    text: String,
    message: &str,
  ) -> Result<()> {
    self.reader.store('"', text)?;
    self.reader.notice(message);
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

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn completion_tracks_document_visibility() -> Result<()> {
    use crossterm::event::KeyModifiers;
    use vpdf_ui::layout::Screen;
    let path =
      std::path::PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
    let screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    for covered in [false, true] {
      let mut app = App::with_screen(&path, "", false, screen)?;
      if covered {
        app.panel = Some(crate::panel::PanelSession::recent(
          Vec::new(),
          &[],
          "",
          &app.wake,
        ));
      }
      assert_eq!(app.covered(), covered);
      let mut prompt = Prompt::new(PromptKind::Command);
      prompt.replace("re");
      app.prompt = Some(prompt);
      app.key(
        KeyEvent::new(crossterm::event::KeyCode::Tab, KeyModifiers::NONE),
        &mut History::default(),
      )?;
      assert_eq!(
        app.prompt.as_ref().unwrap().text(),
        if covered { "recent" } else { "re" }
      );
    }
    Ok(())
  }
}
