use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy)]
pub enum PromptKind {
  Search(bool),
  Command,
  Password,
}

pub struct Prompt {
  pub kind: PromptKind,
  pub text: String,
}

pub enum PromptEvent {
  Pending,
  Submit,
  Cancel,
}

impl Prompt {
  pub fn new(kind: PromptKind) -> Self {
    Self { kind, text: String::new() }
  }

  pub fn display(&self) -> String {
    match self.kind {
      PromptKind::Search(false) => format!("/{}", self.text),
      PromptKind::Search(true) => format!("?{}", self.text),
      PromptKind::Command => format!(":{}", self.text),
      PromptKind::Password => {
        format!("Password: {}", "*".repeat(self.text.chars().count()))
      }
    }
  }

  pub fn feed(&mut self, key: KeyEvent) -> PromptEvent {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
      match key.code {
        KeyCode::Char('c') => return PromptEvent::Cancel,
        KeyCode::Char('u') => self.text.clear(),
        KeyCode::Char('w') => {
          while self.text.ends_with(' ') {
            self.text.pop();
          }
          while !self.text.is_empty() && !self.text.ends_with(' ') {
            self.text.pop();
          }
        }
        _ => {}
      }
      return PromptEvent::Pending;
    }
    match key.code {
      KeyCode::Esc => return PromptEvent::Cancel,
      KeyCode::Enter => return PromptEvent::Submit,
      KeyCode::Backspace => {
        self.text.pop();
      }
      KeyCode::Char(ch) if !ch.is_control() && self.text.len() < 4096 => {
        self.text.push(ch);
      }
      _ => {}
    }
    PromptEvent::Pending
  }
}

pub fn password() -> crate::Result<String> {
  use crossterm::event::{self, Event, KeyEventKind};
  use std::io::{self, Write};
  let mut prompt = Prompt::new(PromptKind::Password);
  let mut out = io::stdout();
  loop {
    let screen = crate::terminal::size()?;
    crate::terminal::line(
      &mut out,
      screen.rows - 1,
      &prompt.display(),
      screen.columns,
    )?;
    out.flush()?;
    if let Event::Key(key) = event::read()?
      && key.kind != KeyEventKind::Release
    {
      match prompt.feed(key) {
        PromptEvent::Submit => return Ok(prompt.text),
        PromptEvent::Cancel => anyhow::bail!("Password cancelled"),
        PromptEvent::Pending => {}
      }
    }
  }
}
