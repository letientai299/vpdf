use crate::Result;
use crate::cmdline;
use crate::keys::{Action, Keys, PanelKind};
use crate::layout::Screen;
use crate::panel::{Destination, Panel, Row, recent_rows};
use crate::prompt::{History, Prompt, PromptEvent, PromptKind};
use crate::state::Entry;
use crate::statusline;
use crate::terminal;
use crate::theme::{self, Color};
use crossterm::event::{Event, KeyEvent, KeyEventKind};
use std::io::Write;
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

/// What the start screen asks the session to do.
#[derive(Debug, Eq, PartialEq)]
pub enum Outcome {
  Stay,
  Open(PathBuf),
  Quit,
}

/// Start screen: recent documents and a command line to open files.
pub struct Home {
  pub screen: Screen,
  pub error: Option<String>,
  panel: Panel,
  prompt: Option<Prompt>,
  keys: Keys,
}

impl Home {
  pub fn new(recent: &[Entry], screen: Screen) -> Self {
    let mut rows = recent_rows(recent);
    if rows.is_empty() {
      rows.push(Row::new(
        "No recent documents; type :e <file>".into(),
        String::new(),
        Destination::None,
      ));
    }
    Self {
      screen,
      error: None,
      panel: Panel::new(PanelKind::Recent, rows),
      prompt: None,
      keys: Keys::default(),
    }
  }

  pub fn draw(&mut self, out: &mut impl Write) -> Result<()> {
    write!(out, "\x1b[?2026h{}", theme::RESET)?;
    for row in 0..self.screen.rows {
      terminal::line(out, row, "", self.screen.columns)?;
    }
    let width = self.screen.columns.saturating_add(1);
    self.panel.draw(out, width, self.screen.rows)?;
    let row = self.screen.rows - 1;
    if let Some(prompt) = &self.prompt {
      statusline::prompt(out, prompt, self.screen)?;
    } else {
      let (text, style) = match &self.error {
        Some(error) => (error.as_str(), theme::ERROR),
        None => ("Enter open  :e file  :q quit", theme::BAR),
      };
      let columns = usize::from(self.screen.columns);
      let text = terminal::clean(text, columns.saturating_sub(8));
      let fill = columns.saturating_sub(7 + text.width());
      write!(
        out,
        "\x1b[{};1H{} VPDF {style} {text}{}{}\x1b[?25l",
        row + 1,
        theme::badge(Color::Blue),
        " ".repeat(fill),
        theme::RESET
      )?;
    }
    write!(out, "\x1b[?2026l")?;
    out.flush()?;
    Ok(())
  }

  pub fn event(
    &mut self,
    event: Event,
    history: &mut History,
  ) -> Result<Outcome> {
    match event {
      Event::Key(key) if key.kind != KeyEventKind::Release => {
        self.error = None;
        self.key(key, history)
      }
      Event::Paste(text) => {
        if let Some(prompt) = &mut self.prompt {
          prompt.insert(&text);
        }
        Ok(Outcome::Stay)
      }
      Event::Resize(_, _) => {
        self.screen = terminal::size().unwrap_or(self.screen);
        Ok(Outcome::Stay)
      }
      Event::Mouse(mouse) if mouse.row < self.screen.rows - 1 => {
        Ok(match self.panel.pointer(mouse) {
          Some(_) => self.selected(),
          None => Outcome::Stay,
        })
      }
      _ => Ok(Outcome::Stay),
    }
  }

  fn key(&mut self, key: KeyEvent, history: &mut History) -> Result<Outcome> {
    let Some(prompt) = &mut self.prompt else {
      let Some(input) = self.keys.feed(key) else { return Ok(Outcome::Stay) };
      if self.panel.navigate(input, usize::from(self.screen.rows)) {
        return Ok(Outcome::Stay);
      }
      return Ok(match input.action {
        Action::Follow | Action::Swap => self.selected(),
        Action::Command => {
          self.prompt = Some(Prompt::new(PromptKind::Command));
          Outcome::Stay
        }
        Action::Quit => Outcome::Quit,
        _ => Outcome::Stay,
      });
    };
    match prompt.feed(key, history) {
      PromptEvent::Complete => {
        let (start, items) = cmdline::candidates(prompt.before_cursor());
        prompt.complete(start, items);
      }
      PromptEvent::Cancel => self.prompt = None,
      PromptEvent::Submit => {
        let text = prompt.text.clone();
        self.prompt = None;
        history.push(PromptKind::Command, &text);
        return command(&text);
      }
      PromptEvent::Pending | PromptEvent::Edited => {}
    }
    Ok(Outcome::Stay)
  }

  fn selected(&self) -> Outcome {
    match self.panel.rows.get(self.panel.selected).map(|r| &r.destination) {
      Some(Destination::File(path)) => Outcome::Open(path.clone()),
      _ => Outcome::Stay,
    }
  }
}

/// Commands that make sense before any document is open.
fn command(line: &str) -> Result<Outcome> {
  let line = line.trim();
  let (name, argument) = line
    .split_once(char::is_whitespace)
    .map_or((line, ""), |(name, rest)| (name, rest.trim()));
  if name.is_empty() {
    return Ok(Outcome::Stay);
  }
  let spec = cmdline::lookup(name)
    .ok_or_else(|| anyhow::anyhow!("Unknown command: {name}"))?;
  match spec.name {
    "edit" | "tabedit" => {
      anyhow::ensure!(!argument.is_empty(), "File required");
      Ok(Outcome::Open(cmdline::expand(argument)))
    }
    "quit" | "qall" | "tabclose" => Ok(Outcome::Quit),
    "recent" => Ok(Outcome::Stay),
    name => anyhow::bail!("Open a document before :{name}"),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn start_commands_open_or_quit() -> Result<()> {
    assert_eq!(command("e a.pdf")?, Outcome::Open("a.pdf".into()));
    assert_eq!(command("tabnew  b.pdf ")?, Outcome::Open("b.pdf".into()));
    assert_eq!(command("q")?, Outcome::Quit);
    assert_eq!(command("")?, Outcome::Stay);
    assert!(command("e").is_err());
    assert!(command("toc").is_err());
    Ok(())
  }
}
