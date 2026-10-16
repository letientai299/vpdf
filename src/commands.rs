use crate::Result;
use crate::app::App;
use crate::document::{Document, Target};
use crate::find::Search;
use crate::graphics::Graphics;
use crate::keys::{Action, Input, PanelKind};
use crate::prompt::{Prompt, PromptEvent, PromptKind};
use crate::render_worker::RenderWorker;
use crate::text::Position;
use base64::{Engine, engine::general_purpose::STANDARD};
use crossterm::event::KeyEvent;
use std::io::Write;
use std::process::{Command, Stdio};

impl App {
  pub fn action(&mut self, input: Input) -> Result<()> {
    if input.action == Action::Cancel {
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
      Action::Search(backward) => {
        self.prompt = Some(Prompt::new(PromptKind::Search(backward)));
      }
      Action::WordSearch => self.word_search()?,
      Action::Result(reverse) => self.next_result(reverse, input.count)?,
      Action::Follow => self.follow_cursor()?,
      Action::Command => self.prompt = Some(Prompt::new(PromptKind::Command)),

      _ => {
        if !self.navigate(input)? {
          self.viewport(input)?;
        }
      }
    }
    Ok(())
  }

  pub fn key(&mut self, key: KeyEvent) -> Result<()> {
    if let Some(prompt) = &mut self.prompt {
      match prompt.feed(key) {
        PromptEvent::Pending => {}
        PromptEvent::Cancel => {
          self.prompt = None;
        }
        PromptEvent::Submit => {
          let Some(prompt) = self.prompt.take() else { return Ok(()) };
          match prompt.kind {
            PromptKind::Search(backward) => {
              self.start_search(&prompt.text, backward, false)?;
            }
            PromptKind::Command => self.command(&prompt.text)?,
            PromptKind::Password => self.reload_with(&prompt.text)?,
          }
        }
      }
    } else if let Some(input) = self.keys.feed(key) {
      self.action(input)?;
    }
    Ok(())
  }

  pub fn command(&mut self, command: &str) -> Result<()> {
    let mut words = command.split_whitespace();
    let name = words.next().unwrap_or("");
    match name {
      "q" | "q!" | "quit" | "bd" => self.quit = true,
      "PdfPage" | "page" => {
        let label =
          words.next().ok_or_else(|| anyhow::anyhow!("Page required"))?;
        let page = self
          .doc
          .page_number(label)
          .ok_or_else(|| anyhow::anyhow!("Page label not found"))?;
        self
          .reader
          .jump(Position { page, ..Position::default() }, &mut self.doc)?;
        self.reader.scroll = self.layout.pages[self.reader.cursor.page.0].top;
        self.ensure_cursor()?;
      }
      "PdfReload" | "reload" => self.reload_with(&self.password.clone())?,
      "PdfInfo" | "info" => self.open_panel(PanelKind::Info)?,
      "PdfText" | "text" => self.open_panel(PanelKind::Text)?,
      "PdfReadme" | "help" => self.open_panel(PanelKind::Help)?,
      "PdfPassword" | "password" => {
        self.prompt = Some(Prompt::new(PromptKind::Password));
      }
      "PdfOcr" | "ocr" => self.start_ocr(words.next().unwrap_or("eng"))?,
      "nohlsearch" | "noh" => self.search.hidden = true,
      "" => {}
      _ if name.chars().all(|ch| ch.is_ascii_digit()) => {
        self.command(&format!("page {name}"))?;
      }
      _ => self.reader.message = "Unknown command".into(),
    }
    Ok(())
  }

  pub fn reload_with(&mut self, password: &str) -> Result<()> {
    let doc = Document::open(&self.path, password)?;
    self.doc = doc;
    self.password = password.to_owned();
    self.worker = RenderWorker::new(&self.path, password);
    self.search = Search::default();
    self.graphics = Graphics::default();
    self.reset_images = true;
    if let Some(ocr) = self.ocr.take() {
      ocr.cancel();
    }
    self.reader.cancel();
    self.reader.clear_history();
    self.requested.clear();
    self.panel = None;
    self.left = 0;
    self.reader.cursor.page.0 =
      self.reader.cursor.page.0.min(self.doc.pages.len() - 1);
    self.reader.cursor =
      self.doc.text(self.reader.cursor.page)?.clamp(self.reader.cursor);
    self.resize()?;
    self.reader.message = "Reloaded".into();
    Ok(())
  }

  pub fn follow_cursor(&mut self) -> Result<()> {
    let cursor = self.reader.cursor;
    let text = self.doc.text(cursor.page)?;
    let Some(glyph) = text.glyph(cursor) else {
      return self.open_panel(PanelKind::Links);
    };
    let x = (glyph.bounds.x0 + glyph.bounds.x1) / 2.0;
    let y = (glyph.bounds.y0 + glyph.bounds.y1) / 2.0;
    let links = self.doc.links(cursor.page)?;
    if let Some(link) = links.iter().find(|link| link.bounds.contains(x, y)) {
      self.follow(&link.target)?;
    } else {
      self.open_panel(PanelKind::Links)?;
    }
    Ok(())
  }

  pub fn follow(&mut self, target: &Target) -> Result<()> {
    match target {
      Target::Page(page) => {
        self.reader.jump(
          Position { page: *page, ..Position::default() },
          &mut self.doc,
        )?;
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

  pub fn clipboard(&mut self, out: &mut impl Write) -> Result<()> {
    let Some(yank) = self.reader.pending_yank.take() else { return Ok(()) };
    if matches!(yank.register, '+' | '*') {
      let data = STANDARD.encode(&yank.text);
      if std::env::var_os("TMUX").is_some() {
        write!(out, "\x1bPtmux;\x1b\x1b]52;c;{data}\x07\x1b\\")?;
      } else {
        write!(out, "\x1b]52;c;{data}\x07")?;
      }
    }
    Ok(())
  }
}
