use crate::Result;
use crate::app::{App, Request, TabMove, Window};
use crate::cmdline;
use crate::document::{Document, Target};
use crate::keys::{Action, Input, PanelKind};
use crate::prompt::{History, Prompt, PromptEvent, PromptKind};
use crate::reader;
use crate::render_worker::RenderWorker;
use crate::split::Axis;
use crate::text::Position;
use crossterm::event::KeyEvent;
use std::process::{Command, Stdio};

impl App {
  pub fn action(&mut self, input: Input) -> Result<()> {
    if self.overview.is_some() && self.overview_input(input)? {
      return Ok(());
    }
    if input.action == Action::Cancel {
      self.animation = None;
      self.search.cancel();
      self.reader.cancel();
      if let Some(ocr) = self.ocr.take() {
        ocr.cancel();
      }
      if let Some(panel) = &mut self.panel {
        panel.focused = false;
      }
      return Ok(());
    }
    // `q` leaves text preview first and keeps any side panel open.
    if matches!(input.action, Action::Quit) && self.preview.is_some() {
      return self.close_text_preview();
    }
    if self.panel_input(input)? {
      return Ok(());
    }
    match input.action {
      Action::Panel(kind) => self.open_panel(kind)?,
      Action::ClosePanel => self.close_panel()?,
      Action::Search(backward) => self.begin_search(backward),
      Action::WordSearch => self.word_search()?,
      Action::Result(reverse) => self.next_result(reverse, input.count)?,
      Action::Follow => self.follow_cursor()?,
      Action::Command => self.prompt = Some(Prompt::new(PromptKind::Command)),
      Action::Mark(name) => self.set_mark(name),
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
    if self.hints.is_some() {
      return self.hint_key(key);
    }
    if let Some(peek) = self.peek.take() {
      // Any key closes the preview.
      let Some(input) = self.keys.feed(key) else { return Ok(()) };
      return match input.action {
        Action::Follow => self.follow(&peek.target),
        Action::Quit | Action::Cancel | Action::Peek => Ok(()),
        _ => self.action(input),
      };
    }
    if self.prompt.is_none() {
      if let Some(input) = self.keys.feed(key) {
        self.action(input)?;
      }
      return Ok(());
    }
    self.edit_prompt(history, |prompt, history| {
      cmdline::feed(prompt, key, history)
    })
  }

  /// Pasted text goes to an open prompt.
  pub fn paste(&mut self, text: &str, history: &mut History) -> Result<()> {
    self.edit_prompt(history, |prompt, _| Ok(prompt.paste(text)))
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
      PromptEvent::Cancel => self.cancel_prompt()?,
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
    if name.chars().all(|ch| ch.is_ascii_digit()) {
      return self.page(name);
    }
    let spec = cmdline::lookup(name)
      .ok_or_else(|| anyhow::anyhow!("Unknown command: {name}"))?;
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
      "edit" | "tabedit" => {
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
      "page" => self.page(argument)?,
      "toc" => self.open_panel(PanelKind::Outline)?,
      "overview" => self.open_overview()?,
      "crop" => self.toggle_crop()?,
      "recent" => self.open_panel(PanelKind::Recent)?,
      "marks" => self.open_panel(PanelKind::Marks)?,
      "annotations" => self.open_panel(PanelKind::Annotations)?,
      "fzf" => self.request = Some(Request::Pick),
      "nohlsearch" => self.search.hidden = true,
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

  fn page(&mut self, label: &str) -> Result<()> {
    anyhow::ensure!(!label.is_empty(), "Page required");
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
    self.load_outline();
    self.graphics.forget(self.id);
    if let Some(ocr) = self.ocr.take() {
      ocr.cancel();
    }
    self.reader.cancel();
    self.reader.clear_history();
    self.requested.clear();
    self.reader.cursor = reader::clamp(self.reader.cursor, &mut self.doc)?;
    if let Some(origin) = &mut self.search_origin {
      origin.cursor = reader::clamp(origin.cursor, &mut self.doc)?;
    }
    if let Some(panel) = &self.panel {
      let kind = panel.kind;
      let rows = self.panel_rows(kind)?;
      // These rows come from the session or refill when work finishes.
      let refills = match kind {
        PanelKind::Recent | PanelKind::Marks => true,
        PanelKind::Results => rerun,
        PanelKind::Outline => self.outline_job.is_some(),
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
    // New pages can move the cursor's glyph.
    self.ensure_cursor()?;
    self.reader.message = "Reloaded".into();
    Ok(())
  }

  /// Enter follows a link under the cursor and ignores plain text.
  pub fn follow_cursor(&mut self) -> Result<()> {
    match self.link_at_cursor()? {
      Some(target) => self.follow(&target),
      None => Ok(()),
    }
  }

  /// Destination of the link covering the cursor glyph.
  pub fn link_at_cursor(&mut self) -> Result<Option<Target>> {
    let cursor = self.reader.cursor;
    let text = self.doc.text(cursor.page)?;
    let Some(glyph) = text.glyph(cursor) else { return Ok(None) };
    let x = f32::midpoint(glyph.bounds.x0, glyph.bounds.x1);
    let y = f32::midpoint(glyph.bounds.y0, glyph.bounds.y1);
    let links = self.doc.links(cursor.page)?;
    Ok(
      links
        .into_iter()
        .find(|link| link.bounds.contains(x, y))
        .map(|link| link.target),
    )
  }

  pub fn follow(&mut self, target: &Target) -> Result<()> {
    match target {
      Target::Page(page) | Target::Point { page, .. } => {
        let page = crate::text::PageId(page.0.min(self.doc.pages.len() - 1));
        let position = match target {
          Target::Point { x, y, .. } => {
            self.doc.text(page)?.nearest(page, *x, *y)
          }
          _ => Position { page, ..Position::default() },
        };
        self.go_to(position, true)?;
      }
      Target::Uri(uri) => {
        anyhow::ensure!(
          uri.starts_with("https://") || uri.starts_with("http://"),
          "Unsupported link scheme"
        );
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
      }
    }
    Ok(())
  }
}
