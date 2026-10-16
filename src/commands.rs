use crate::Result;
use crate::app::{App, Request, TabMove};
use crate::cmdline;
use crate::document::{Document, Target};
use crate::keys::{Action, Input, PanelKind};
use crate::prompt::{History, Prompt, PromptEvent, PromptKind};
use crate::render_worker::RenderWorker;
use crate::text::Position;
use crossterm::event::KeyEvent;
use std::process::{Command, Stdio};

impl App {
  pub fn action(&mut self, input: Input) -> Result<()> {
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
    if self.panel_input(input)? {
      return Ok(());
    }
    if matches!(input.action, Action::Quit) && self.preview.is_some() {
      return self.close_panel();
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
      Action::Peek => self.preview_link()?,
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
      // Any key closes the preview; Enter also follows it.
      match key.code {
        crossterm::event::KeyCode::Enter => return self.follow(&peek.target),
        crossterm::event::KeyCode::Char('K')
        | crossterm::event::KeyCode::Esc => {
          return Ok(());
        }
        _ => {}
      }
    }
    let Some(prompt) = &mut self.prompt else {
      if let Some(input) = self.keys.feed(key) {
        self.action(input)?;
      }
      return Ok(());
    };
    let menu = prompt.menu.is_some();
    let event = prompt.feed(key, history);
    let kind = prompt.kind;
    match event {
      PromptEvent::Edited if matches!(kind, PromptKind::Search(_)) => {
        let query = prompt.text.clone();
        self.incremental(&query)?;
      }
      PromptEvent::Pending | PromptEvent::Edited => {}
      PromptEvent::Complete => {
        if prompt.kind == PromptKind::Command {
          let (start, items) = cmdline::candidates(prompt.before_cursor());
          prompt.complete(start, items);
        }
      }
      PromptEvent::Cancel => {
        self.prompt = None;
        if matches!(kind, PromptKind::Search(_)) {
          self.abort_search()?;
        }
      }
      PromptEvent::Submit => {
        let Some(prompt) = self.prompt.take() else { return Ok(()) };
        history.push(prompt.kind, &prompt.text);
        self.relayout()?;
        match prompt.kind {
          PromptKind::Search(backward) => {
            self.finish_search(&prompt.text, backward)?;
          }
          PromptKind::Command => self.command(&prompt.text)?,
          PromptKind::Password => self.reload_with(&prompt.text)?,
        }
        return Ok(());
      }
    }
    if menu != self.prompt.as_ref().is_some_and(|p| p.menu.is_some()) {
      self.relayout()?;
    }
    Ok(())
  }

  pub fn command(&mut self, line: &str) -> Result<()> {
    let line = line.trim();
    let (name, argument) = line
      .split_once(char::is_whitespace)
      .map_or((line, ""), |(name, rest)| (name, rest.trim()));
    if name.is_empty() {
      return Ok(());
    }
    if name.chars().all(|ch| ch.is_ascii_digit()) {
      return self.page(name);
    }
    let spec = cmdline::lookup(name)
      .ok_or_else(|| anyhow::anyhow!("Unknown command: {name}"))?;
    match spec.name {
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
        self.start_ocr(if argument.is_empty() { "eng" } else { argument })?
      }
      "help" => self.open_panel(PanelKind::Help)?,
      "quit" | "tabclose" => self.quit = true,
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
    self
      .reader
      .jump(Position { page, ..Position::default() }, &mut self.doc)?;
    self.reader.scroll = self.layout.pages[self.reader.cursor.page.0].top;
    self.ensure_cursor()
  }

  pub fn reload_with(&mut self, password: &str) -> Result<()> {
    let doc = Document::open(&self.path, password)?;
    self.doc = doc;
    self.password = password.to_owned();
    self.worker = RenderWorker::new(&self.path, password);
    let previous = std::mem::take(&mut self.search);
    self.stamp = crate::watch::stamp(&self.path);
    self.infer_outline();
    self.graphics.forget(self.id);
    if let Some(ocr) = self.ocr.take() {
      ocr.cancel();
    }
    self.reader.cancel();
    self.reader.clear_history();
    self.requested.clear();
    self.reader.cursor.page.0 =
      self.reader.cursor.page.0.min(self.doc.pages.len() - 1);
    self.reader.cursor =
      self.doc.text(self.reader.cursor.page)?.clamp(self.reader.cursor);
    if let Some(panel) = &self.panel {
      let (kind, selected) = (panel.kind, panel.selected);
      let rows = self.panel_rows(kind)?;
      if let Some(panel) = self.panel.as_mut().filter(|_| !rows.is_empty()) {
        panel.selected = selected.min(rows.len() - 1);
        panel.rows = rows;
      } else {
        self.panel = None;
        self.left = 0;
      }
    }
    if !previous.hidden && !previous.query.is_empty() {
      self.start_search(&previous.query, previous.backward, false)?;
      self.search.phase = crate::find::Phase::Running { jump: false };
    }
    self.resize()?;
    self.reader.message = "Reloaded".into();
    Ok(())
  }

  pub fn follow_cursor(&mut self) -> Result<()> {
    match self.link_at_cursor()? {
      Some(target) => self.follow(&target),
      None => self.open_panel(PanelKind::Links),
    }
  }

  /// Destination of the link covering the cursor glyph.
  pub fn link_at_cursor(&mut self) -> Result<Option<Target>> {
    let cursor = self.reader.cursor;
    let text = self.doc.text(cursor.page)?;
    let Some(glyph) = text.glyph(cursor) else { return Ok(None) };
    let x = (glyph.bounds.x0 + glyph.bounds.x1) / 2.0;
    let y = (glyph.bounds.y0 + glyph.bounds.y1) / 2.0;
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
        self.reader.jump(position, &mut self.doc)?;
        self.reader.scroll = self.layout.pages[self.reader.cursor.page.0].top;
        self.ensure_cursor()?;
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
