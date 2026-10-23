use crate::Result;
use crate::app::{App, Request};
use crate::cmdline;
use crate::pdf_text::PdfText;
use std::process::{Command, Stdio};
use vpdf_model::document::Target;
use vpdf_model::input::KeyEvent;
use vpdf_model::text::{PageId, Position};
use vpdf_reader::reader::Mode;
use vpdf_ui::bindings::Context;
use vpdf_ui::commands::{self, Id, View};
use vpdf_ui::input::{Cancel, InputOwner, Reset};
use vpdf_ui::keys::{Action, Dispatch, Input, PanelKind};
use vpdf_ui::prompt::{History, Prompt, PromptEvent, PromptKind};
use vpdf_ui::split::Axis;
use vpdf_ui::window::{TabMove, Window};

/// The note after copying an address.
pub(crate) const COPIED_LINK: &str = "Copied link";

impl App {
  pub(crate) fn help_invocation(&self) -> vpdf_ui::topics::Invocation {
    vpdf_ui::topics::Invocation::Pane {
      context: Context::resolve(
        self.input_owner(),
        self.reader.mode(),
        self.reader.operator.is_some(),
      ),
      pane: vpdf_ui::topics::PaneContext {
        id: self.id,
        document: self.document_identity().display().to_string(),
        document_generation: self.document_generation,
        position: self.navigation_position(),
        zoom: self.layout.zoom(),
        cursorline: self.cursorline,
        text_mode: self.text_mode,
        panel: self
          .panel
          .as_ref()
          .filter(|panel| panel.view.focused())
          .map(|panel| panel.view.kind()),
      },
    }
  }

  pub fn action(&mut self, input: Input) -> Result<()> {
    let invocation = self.help_invocation();
    if input.action == Action::Help {
      self.request = Some(Request::Help { topic: String::new(), invocation });
      return Ok(());
    }
    self.cancel_definition();
    self.search.forget_yank();
    if self.reader.prepare(input.action) {
      if self.keys.resizing() {
        self.reader.disarm();
      } else {
        self.disarm();
      }
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
      && self.overlays.overview().is_some()
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
      Action::Command => {
        self.overlays.open_prompt(
          Prompt::new(PromptKind::Command).with_invocation(invocation),
        );
      }
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
      Action::RegionStart => self.start_region()?,
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
    // Keys end pointer gestures.
    let swept = self.end_gestures();
    self.drop_hover_preview();
    if let Some(prompt) = self.overlays.prompt_mut() {
      prompt.set_bindings(
        self.settings.bindings().map(Context::prompt(prompt.kind())),
      );
      let view = self.command_view();
      let terminal = matches!(self.frontend, crate::raster::Frontend::Terminal);
      let invocation = self
        .overlays
        .prompt()
        .and_then(Prompt::invocation)
        .cloned()
        .unwrap_or_else(|| self.help_invocation());
      let settings = self.settings.clone();
      return self.edit_prompt(history, |prompt, history| {
        cmdline::feed(prompt, key, history, view, terminal, |query| {
          crate::manual::completions(&settings, &invocation, query)
        })
      });
    }
    if self.overlays.hints().is_some() && self.hint_key(key)? {
      return Ok(());
    }
    let owner = self.input_owner();
    let context = Context::resolve(
      owner,
      self.reader.mode(),
      self.reader.operator.is_some(),
    );
    let dispatch = self.keys.feed_with(key, context, self.settings.bindings());
    if matches!(
      owner,
      InputOwner::Context | InputOwner::Peek | InputOwner::Region
    ) && (self.keys.resizing()
      || matches!(self.keys.awaiting(), Some("C-w")))
    {
      self.dismiss_overlay();
    }
    let Some(dispatch) = dispatch else {
      if self.keys.pending().is_empty() {
        self.reader.disarm();
      }
      return Ok(());
    };
    let mut input = match dispatch {
      Dispatch::Command(command) => {
        let invocation = self.help_invocation();
        self.disarm();
        return self.command_from(&command, Some(invocation));
      }
      Dispatch::Input(input) => input,
    };
    if swept && input.action == Action::Cancel {
      self.disarm();
      return Ok(());
    }
    if input.action == Action::Cancel {
      return self.cancel_input();
    }
    let invocation =
      (input.action == Action::Command).then(|| self.help_invocation());
    if !matches!(owner, InputOwner::Context | InputOwner::Peek)
      && let Action::Panel(kind) = input.action
      && self.panel_focused()
      && self.panel.as_ref().is_some_and(|panel| panel.view.kind() == kind)
    {
      input.action = Action::ClosePanel;
    }
    if input.action == Action::Help {
      return self.action(input);
    }
    let result = match owner {
      InputOwner::Context => self.context_input(input),
      InputOwner::Peek => self.preview_input(input),
      _ => self.action(input),
    };
    if let (Some(invocation), Some(prompt)) =
      (invocation, self.overlays.prompt_mut())
    {
      prompt.set_invocation(invocation);
    }
    result
  }

  pub(crate) fn cancel_input(&mut self) -> Result<()> {
    if self.keys.resizing() {
      self.keys.clear();
      return Ok(());
    }
    self.cancel_definition();
    self.reader.cancel_scroll();
    let owner = self.input_owner();
    let pending = self.keys.cancelled_pending()
      || !self.keys.pending().is_empty()
      || self.reader.operator.is_some();
    let cancel = self.overlays.cancel(owner, pending);
    match cancel {
      Cancel::Disarm => {}
      Cancel::Overlay => self.dismiss_overlay(),
      Cancel::Prompt => self.cancel_prompt()?,
      Cancel::Reset(reset) => {
        self.overlays.close_region();
        self.end_gestures();
        match reset {
          Reset::Prompt => self.cancel_prompt()?,
          Reset::Hints => drop(self.overlays.close_hints()),
          Reset::Overview => self.close_overview(),
          Reset::Pending => self.reader.notice(""),
          Reset::Content => self.reader.cancel_content(),
          Reset::Selection => self.reader.cancel(),
        }
      }
    }
    self.disarm();
    Ok(())
  }

  /// Pasted text goes to an open prompt. In Normal mode a pasted PDF
  /// path opens, since terminals without drops paste dropped paths.
  pub fn paste(&mut self, text: &str, history: &mut History) -> Result<()> {
    if self.overlays.prompt().is_some() {
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
    let Some(prompt) = self.overlays.prompt_mut() else { return Ok(()) };
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
      PromptEvent::Command => {
        if let Some(command) = prompt.take_command() {
          self.command(&command)?;
        }
      }
      PromptEvent::Submit => {
        let Some(prompt) = self.overlays.close_prompt() else { return Ok(()) };
        history.push(prompt.kind(), prompt.text());
        match prompt.kind() {
          PromptKind::Search(backward) => {
            self.finish_search(prompt.text(), backward)?;
          }
          PromptKind::Command => {
            self.command_from(prompt.text(), prompt.invocation().cloned())?;
          }
          PromptKind::Password => self.reload_with(prompt.text())?,
        }
      }
    }
    Ok(())
  }

  /// Closes the prompt; searches return.
  pub fn cancel_prompt(&mut self) -> Result<()> {
    let Some(prompt) = self.overlays.close_prompt() else { return Ok(()) };
    if matches!(prompt.kind(), PromptKind::Search(_)) {
      self.abort_search()?;
    }
    Ok(())
  }

  fn command_view(&self) -> View {
    if self.covered() { View::Recent } else { View::Document }
  }

  pub(crate) fn command_available(&self, program: &str) -> bool {
    cmdline::available(
      program,
      matches!(self.frontend, crate::raster::Frontend::Terminal),
    )
  }

  fn theme_command(&mut self, argument: &str) -> Result<()> {
    self.request = Some(Request::Theme(commands::theme(argument)?));
    Ok(())
  }

  pub fn command(&mut self, line: &str) -> Result<()> {
    let invocation =
      self.overlays.prompt().and_then(Prompt::invocation).cloned();
    self.command_from(line, invocation)
  }

  fn command_from(
    &mut self,
    line: &str,
    invocation: Option<vpdf_ui::topics::Invocation>,
  ) -> Result<()> {
    let (spec, argument) = match commands::resolve(
      line,
      self.command_view(),
      |program| self.command_available(program),
      |name| self.doc.page_number(name).is_some(),
    )? {
      commands::Resolution::Empty => return Ok(()),
      commands::Resolution::Page(name) => return self.page(name),
      commands::Resolution::Command { spec, argument } => (spec, argument),
    };
    match spec.id {
      Id::Split | Id::VSplit => {
        let axis =
          if spec.id == Id::Split { Axis::Rows } else { Axis::Columns };
        let file = (!argument.is_empty()).then(|| cmdline::expand(argument));
        self.request = Some(Request::Window {
          command: Window::Split(axis),
          count: 1,
          explicit: false,
          file,
        });
      }
      Id::Close | Id::Only => {
        let command =
          if spec.id == Id::Close { Window::Close } else { Window::Only };
        self.request = Some(Request::Window {
          command,
          count: 1,
          explicit: false,
          file: None,
        });
      }
      Id::TabNew if argument.is_empty() => {
        self.request = Some(Request::RecentTab);
      }
      Id::Edit | Id::TabEdit | Id::TabNew => {
        anyhow::ensure!(!argument.is_empty(), "File required");
        let path = cmdline::expand(argument);
        self.request = Some(Request::Open { path, tab: spec.id != Id::Edit });
      }
      Id::TabNext => {
        let step = argument.parse().map_or(TabMove::Next(1), TabMove::Go);
        self.request = Some(Request::Tab(step));
      }
      Id::TabPrevious => {
        self.request = Some(Request::Tab(TabMove::Previous(1)));
      }
      Id::QuitAll => self.request = Some(Request::QuitAll),
      Id::Toc => self.open_panel(PanelKind::Outline)?,
      Id::Overview => self.open_overview()?,
      Id::Crop => {
        self.set_crop(spec.switch(argument, self.crop_enabled())?)?;
      }
      Id::Recent => self.open_panel(PanelKind::Recent)?,
      Id::Marks => self.open_panel(PanelKind::Marks)?,
      Id::Jumps => self.open_panel(PanelKind::Jumps)?,
      Id::Annotations => self.open_panel(PanelKind::Annotations)?,
      Id::Fzf => self.request = Some(Request::Pick),
      Id::Cancel => {
        self.cancel_jobs()?;
        self.reader.notice("Jobs canceled");
      }
      Id::NoHlSearch => self.search.set_hidden(true),
      Id::CursorLine => {
        self.cursorline = spec.switch(argument, self.cursorline)?;
      }
      Id::Perf => {
        self.request = Some(Request::Perf(crate::perf::switch(argument)?));
      }
      Id::Mouse => {
        let on = spec.switch(argument, vpdf_terminal::terminal::mouse())?;
        self.request = Some(Request::Mouse(on));
        let message =
          if on { "Mouse on" } else { "Mouse off; :mouse on restores" };
        self.reader.notice(message);
      }
      Id::Text => self.open_panel(PanelKind::Text)?,
      Id::Info => self.open_panel(PanelKind::Info)?,
      Id::Config => self.reload_config(argument)?,
      Id::Theme => self.theme_command(argument)?,
      Id::Reload => self.reload_with(&self.password.clone())?,
      Id::Print => self.print(argument)?,
      Id::Password => {
        self.overlays.open_prompt(Prompt::new(PromptKind::Password));
      }
      Id::Ocr => {
        let language =
          if argument.is_empty() { self.settings.language() } else { argument }
            .to_owned();
        self.start_ocr(&language)?;
      }
      Id::Version => self.open_panel(PanelKind::Version)?,
      Id::Help | Id::HelpBeside => {
        self.request_help(argument, invocation, spec.id == Id::HelpBeside);
      }
      Id::Quit => self.quit = true,
      Id::TabClose => self.request = Some(Request::TabClose(None)),
    }
    Ok(())
  }

  fn request_help(
    &mut self,
    topic: &str,
    invocation: Option<vpdf_ui::topics::Invocation>,
    beside: bool,
  ) {
    let invocation = invocation.unwrap_or_else(|| self.help_invocation());
    self.request = Some(if beside {
      Request::HelpBeside { topic: topic.into(), invocation }
    } else {
      Request::Help { topic: topic.into(), invocation }
    });
  }

  fn reload_config(&mut self, argument: &str) -> Result<()> {
    crate::config::reload_command(argument)?;
    self.request = Some(Request::ConfigReload);
    Ok(())
  }

  fn print(&mut self, argument: &str) -> Result<()> {
    anyhow::ensure!(argument.is_empty(), "Usage: :print");
    self.request = Some(Request::Print);
    Ok(())
  }

  fn yank_path(&mut self, register: char) -> Result<()> {
    let register =
      self.reader.take_operator().map_or(register, |op| op.register);
    self
      .reader
      .store(register, self.document_identity().display().to_string())?;
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
  ) -> Result<Option<vpdf_model::document::Link>> {
    Ok(self.doc.links(page)?.get(index).cloned())
  }

  /// Centre of the glyph at `position`.
  pub(crate) fn glyph_point(
    &mut self,
    position: Position,
  ) -> Result<Option<(PageId, f32, f32)>> {
    vpdf_ui::reading::glyph_point(&mut PdfText(&mut self.doc), position)
  }

  /// Index of the link covering a page point.
  pub(crate) fn link_at(
    &mut self,
    page: PageId,
    x: f32,
    y: f32,
  ) -> Result<Option<usize>> {
    vpdf_ui::reading::link_at(&mut PdfText(&mut self.doc), page, x, y)
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
    use vpdf_model::input::KeyModifiers;
    let path =
      std::path::PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
    let screen = crate::screen::cells(80, 24);
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
      app.overlays.open_prompt(prompt);
      app.key(
        KeyEvent::new(vpdf_model::input::KeyCode::Tab, KeyModifiers::NONE),
        &mut History::default(),
      )?;
      assert_eq!(
        app.overlays.prompt().unwrap().text(),
        if covered { "recent" } else { "re" }
      );
    }
    Ok(())
  }

  #[test]
  fn version_opens_build_panel() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("source.pdf");
    crate::fixtures::pdf(&path, &["first"], "")?;
    let screen = crate::screen::cells(120, 40);
    for frontend in [
      crate::raster::Frontend::Terminal,
      crate::raster::Frontend::Native(vpdf_pdf::jobs::Wake::none()),
    ] {
      let terminal = matches!(frontend, crate::raster::Frontend::Terminal);
      let settings = crate::config::Settings::default();
      let mut app =
        App::configured(&path, "", false, screen, frontend, &settings)?;
      app.command("version")?;
      let panel = &app.panel.as_ref().expect("version panel").view;
      assert_eq!(panel.kind(), PanelKind::Version);
      assert_eq!(
        panel.rows().iter().map(|row| row.label.as_str()).collect::<Vec<_>>(),
        crate::version::text(terminal).lines().collect::<Vec<_>>()
      );
      assert!(!app.quit);
    }
    Ok(())
  }
}

#[cfg(test)]
mod help_context_tests {
  use super::*;
  use vpdf_ui::bindings::Context;

  #[test]
  fn command_keeps_invoking_pane() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("source.pdf");
    crate::fixtures::pdf(&path, &["first"], "")?;
    for frontend in [
      crate::raster::Frontend::Terminal,
      crate::raster::Frontend::Native(vpdf_pdf::jobs::Wake::none()),
    ] {
      let mut app = App::configured(
        &path,
        "",
        false,
        crate::screen::cells(120, 40),
        frontend,
        &crate::config::Settings::default(),
      )?;
      app.cursorline = false;
      app.layout.request_zoom(vpdf_ui::layout::Zoom::To(2.0));
      app.action(Input::plain(Action::Cursor))?;
      app.action(Input::plain(Action::Command))?;
      let invocation =
        app.overlays.prompt().unwrap().invocation().unwrap().clone();
      assert_eq!(invocation.context(), Context::Cursor);
      let pane = invocation.pane().unwrap();
      assert_eq!(pane.id, app.id);
      assert_eq!(pane.zoom, 2.0);
      assert!(!pane.cursorline);
      assert_eq!(app.input_owner(), InputOwner::Prompt);
      app.cancel_prompt()?;
      app.action(Input::plain(Action::Yank))?;
      assert!(app.reader.operator.is_some());
      app.action(Input::plain(Action::Command))?;
      assert_eq!(
        app.overlays.prompt().unwrap().invocation().unwrap().context(),
        Context::Operator
      );
      assert!(app.reader.operator.is_none());
      app.cancel_prompt()?;
      app.open_panel(PanelKind::Outline)?;
      app.action(Input::plain(Action::Command))?;
      assert_eq!(
        app.overlays.prompt().unwrap().invocation().unwrap().context(),
        Context::Panel
      );
      let invocation = app.overlays.prompt().unwrap().invocation().unwrap();
      assert_eq!(invocation.pane().unwrap().panel, Some(PanelKind::Outline));
      let reference = app.settings.help_reference(invocation.clone());
      let registry = vpdf_ui::topics::Registry::new(
        vpdf_ui::topics::authored(),
        reference.bindings,
      )?;
      let outline = registry
        .action(&vpdf_ui::topics::TopicId::parse("panel.outline")?)
        .unwrap();
      assert!(
        outline.effective.iter().any(|row| row.context == Context::Panel
          && row.sequence.to_string() == "go"
          && row.purpose == "close panel")
      );
      app.cancel_prompt()?;
      for &key in vpdf_ui::bindings::Sequence::parse("go")?.keys() {
        app.key(key, &mut History::default())?;
      }
      assert!(app.panel.is_none());
    }
    Ok(())
  }

  #[test]
  fn command_captures_menu_before_dismissal() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("source.pdf");
    crate::fixtures::pdf(&path, &["first"], "")?;
    for frontend in [
      crate::raster::Frontend::Terminal,
      crate::raster::Frontend::Native(vpdf_pdf::jobs::Wake::none()),
    ] {
      let mut app = App::configured(
        &path,
        "",
        false,
        crate::screen::cells(120, 40),
        frontend,
        &crate::config::Settings::default(),
      )?;
      app.action(Input::plain(Action::Cursor))?;
      app.overlays.open_menu(vpdf_ui::context::ContextMenu::new(
        Vec::new(),
        (0, 0),
        (0, 120),
        39,
        false,
      ));
      let key = vpdf_ui::bindings::Sequence::parse(":")?.keys()[0];
      app.key(key, &mut History::default())?;
      assert!(app.overlays.menu().is_none());
      assert_eq!(
        app.overlays.prompt().unwrap().invocation().unwrap().context(),
        Context::Menu
      );
    }
    Ok(())
  }
}
