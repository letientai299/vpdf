use crate::Result;
use crate::keys::{Action, Keys};
use crate::layout::Screen;
use crate::panel::Panel;
use crate::panel::recent::{Entry, Shelf};
use crate::panel::{Destination, Row};
use crate::prompt::{History, Prompt, PromptEvent, PromptKind};
use crate::theme::{self, Color};
use crossterm::event::{KeyEvent, KeyEventKind};
use std::io::Write;
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;
use vpdf_terminal::decode::Event;
use vpdf_terminal::graphics::Graphics;
use vpdf_terminal::terminal::{self, At};

#[derive(Debug, Eq, PartialEq)]
pub enum Effect {
  Stay,
  Open(PathBuf),
  Quit,
  Command(String),
  Paste(String),
  Editor(String),
  Complete(String),
}

/// Start screen: recent documents and a command line to open files.
pub struct Home {
  screen: Screen,
  at: At,
  error: Option<String>,
  panel: Panel,
  prompt: Option<Prompt>,
  keys: Keys,
}

impl Home {
  pub fn new(mut rows: Vec<Row>, recent: &[Entry], screen: Screen) -> Self {
    if rows.is_empty() {
      rows.push(Row::new(
        "No recent documents; type :e <file>".into(),
        String::new(),
        Destination::None,
      ));
    }
    let panel = Panel::recent(rows, recent);
    Self {
      screen,
      at: At::default(),
      error: None,
      panel,
      prompt: None,
      keys: Keys::default(),
    }
  }

  pub fn screen(&self) -> Screen {
    self.screen
  }

  pub fn resize(&mut self, screen: Screen, at: At) {
    self.screen = screen;
    self.at = at;
  }

  pub fn error(&self) -> Option<&str> {
    self.error.as_deref()
  }

  pub fn fail(&mut self, error: String) {
    self.error = Some(error);
  }

  pub fn shelf_mut(&mut self) -> Option<&mut Shelf> {
    self.panel.shelf_mut()
  }

  pub fn draw(
    &mut self,
    out: &mut impl Write,
    graphics: &mut Graphics,
    now: u64,
    request: impl FnOnce(&mut Shelf),
  ) -> Result<()> {
    let at = self.at;
    for row in 0..self.screen.rows {
      terminal::clear(out, at, row, self.screen.columns)?;
    }
    let drawn = self.panel.draw(
      out,
      at,
      self.screen.columns,
      self.screen,
      Some(graphics),
      now,
    );
    if let Some(cards) = self.panel.shelf_mut() {
      request(cards);
    }
    drawn?;
    let row = self.screen.rows - 1;
    if let Some(prompt) = &self.prompt {
      prompt.draw(out, self.screen, at, None)?;
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
        "{}{} VPDF {style} {text}{}{}\x1b[?25l",
        at.goto(row, 0u16),
        theme::badge(Color::Blue),
        " ".repeat(fill),
        theme::RESET
      )?;
    }
    Ok(())
  }

  /// Whether a prompt takes typed and pasted text.
  pub fn typing(&self) -> bool {
    self.prompt.is_some()
  }

  pub fn replace_prompt(&mut self, text: &str) {
    if let Some(prompt) = &mut self.prompt {
      prompt.replace(text);
      self.preview_prompt();
    }
  }

  pub fn complete_prompt(&mut self, start: usize, items: Vec<String>) {
    if let Some(prompt) = &mut self.prompt {
      prompt.complete(start, items);
      self.preview_prompt();
    }
  }

  fn preview_prompt(&mut self) {
    if let Some(prompt) = &self.prompt
      && matches!(prompt.kind(), PromptKind::Search(_))
    {
      self.panel.preview(prompt.text());
    }
  }

  pub fn event(
    &mut self,
    event: Event,
    history: &mut History,
  ) -> Result<Effect> {
    match event {
      Event::Key(key) if key.kind != KeyEventKind::Release => {
        self.error = None;
        self.key(key, history)
      }
      Event::Paste(text) => {
        // Terminals without drops paste dropped files' paths.
        let Some(prompt) = &mut self.prompt else {
          return Ok(Effect::Paste(text));
        };
        let event = prompt.paste(&text);
        self.prompt_event(event, history)
      }
      // An open prompt owns the input.
      Event::Mouse(mouse) if self.prompt.is_none() => {
        let (rows, cell) = (self.screen.rows - 1, self.screen.cell());
        let mouse = mouse.cells(cell).and_then(|mut mouse| {
          mouse.row = mouse.row.checked_sub(self.at.row)?;
          mouse.column = mouse.column.checked_sub(self.at.column)?;
          (mouse.row < rows).then_some(mouse)
        });
        let Some(mouse) = mouse else {
          return Ok(Effect::Stay);
        };
        Ok(match self.panel.pointer(mouse) {
          Some(_) => self.selected(),
          None => Effect::Stay,
        })
      }
      _ => Ok(Effect::Stay),
    }
  }

  fn key(&mut self, key: KeyEvent, history: &mut History) -> Result<Effect> {
    let Some(prompt) = &mut self.prompt else {
      let Some(input) = self.keys.feed(key, crate::keys::Grammar::Other) else {
        return Ok(Effect::Stay);
      };
      if self.panel.navigate(input, usize::from(self.screen.rows)) {
        return Ok(Effect::Stay);
      }
      return Ok(match input.action {
        Action::Follow | Action::Swap => self.selected(),
        Action::Command => {
          self.prompt = Some(Prompt::new(PromptKind::Command));
          Effect::Stay
        }
        Action::Search(backward) => {
          self.panel.begin_search(backward);
          self.prompt = Some(Prompt::new(PromptKind::Search(backward)));
          Effect::Stay
        }
        Action::Quit => Effect::Quit,
        _ => Effect::Stay,
      });
    };
    let event = prompt.feed(key, history);
    self.prompt_event(event, history)
  }

  fn prompt_event(
    &mut self,
    event: PromptEvent,
    history: &mut History,
  ) -> Result<Effect> {
    let Some(prompt) = &mut self.prompt else { return Ok(Effect::Stay) };
    let kind = prompt.kind();
    let search = matches!(kind, PromptKind::Search(_));
    match event {
      PromptEvent::Edited => self.preview_prompt(),
      PromptEvent::Editor => {
        return Ok(Effect::Editor(prompt.text().to_owned()));
      }
      PromptEvent::Complete if kind == PromptKind::Command => {
        return Ok(Effect::Complete(prompt.before_cursor().to_owned()));
      }
      PromptEvent::Cancel => {
        self.prompt = None;
        self.panel.abort_search();
      }
      PromptEvent::Submit => {
        let text = prompt.text().to_owned();
        self.prompt = None;
        history.push(kind, &text);
        if !search {
          return Ok(Effect::Command(text));
        }
        anyhow::ensure!(
          self.panel.finish_search(&text),
          "Pattern not found: {}",
          self.panel.query()
        );
      }
      _ => {}
    }
    Ok(Effect::Stay)
  }

  fn selected(&self) -> Effect {
    match self.panel.rows().get(self.panel.selected()).map(|r| &r.destination) {
      Some(Destination::File(path)) => Effect::Open(path.clone()),
      _ => Effect::Stay,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn effects_resume_the_prompt() -> Result<()> {
    use crossterm::event::{KeyCode, KeyModifiers};
    let screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let rows = ["paper.pdf", "book.pdf"]
      .map(|path| {
        Row::new(path.into(), String::new(), Destination::File(path.into()))
      })
      .into();
    let mut home = Home::new(rows, &[], screen);
    let mut history = History::default();
    let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    home.event(key(KeyCode::Char(':')), &mut history)?;
    home.event(Event::Paste("e ".into()), &mut history)?;
    assert_eq!(
      home.event(key(KeyCode::Tab), &mut history)?,
      Effect::Complete("e ".into())
    );
    home.complete_prompt(2, vec!["paper.pdf".into()]);
    assert_eq!(
      home.event(key(KeyCode::Enter), &mut history)?,
      Effect::Command("e paper.pdf".into())
    );
    assert!(!home.typing());
    home.event(key(KeyCode::Char('/')), &mut history)?;
    home.event(
      Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
      &mut history,
    )?;
    assert_eq!(
      home.event(key(KeyCode::Char('e')), &mut history)?,
      Effect::Editor(String::new())
    );
    home.replace_prompt("book");
    assert_eq!(home.panel.selected(), 1);
    assert_eq!(home.event(key(KeyCode::Enter), &mut history)?, Effect::Stay);
    assert_eq!(
      home.event(key(KeyCode::Enter), &mut history)?,
      Effect::Open("book.pdf".into())
    );
    assert_eq!(
      home.event(Event::Paste("paper.pdf".into()), &mut history)?,
      Effect::Paste("paper.pdf".into())
    );
    Ok(())
  }

  #[test]
  fn panel_ends_at_the_last_column() -> Result<()> {
    let screen = Screen {
      columns: 40,
      rows: 10,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut out = Vec::new();
    Home::new(Vec::new(), &[], screen).draw(
      &mut out,
      &mut Graphics::default(),
      0,
      |_| {},
    )?;
    let out = String::from_utf8(out)?;
    let columns = out.split("\x1b[").filter_map(|code| {
      let (row, column) = code.split_once('H')?.0.split_once(';')?;
      row.parse::<u16>().ok()?;
      column.parse::<u16>().ok()
    });
    assert_eq!(columns.max(), Some(screen.columns));
    for row in 0..screen.rows - 1 {
      let goto = At::default().goto(row, screen.columns - 1);
      let border = format!("{goto}{}│", theme::MUTED);
      assert!(out.contains(&border), "border on row {row}");
    }
    Ok(())
  }

  #[test]
  fn top_stats_offset_home_and_mouse() -> Result<()> {
    use crossterm::event::KeyModifiers;
    use vpdf_terminal::decode::{Button, Mouse, MouseKind, Spot};
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let rows = || {
      vec![Row::new(file.into(), String::new(), Destination::File(file.into()))]
    };
    let screen = Screen {
      columns: 80,
      rows: 20,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 4),
    };
    let mut home = Home::new(rows(), &[], screen);
    home.at.row = 4;
    let mut out = Vec::new();
    home.draw(&mut out, &mut Graphics::default(), 0, |_| {})?;
    let text = String::from_utf8(out)?;
    assert!(text.contains("\x1b[5;1H"));
    assert!(!text.contains("\x1b[1;1H"));
    for (row, expected) in [(1, Effect::Stay), (5, Effect::Open(file.into()))] {
      let click = Event::Mouse(Mouse {
        kind: MouseKind::Press(Button::LEFT),
        at: Spot::Cell { column: 5, row },
        modifiers: KeyModifiers::NONE,
      });
      assert_eq!(home.event(click, &mut History::default())?, expected);
    }
    Ok(())
  }

  #[test]
  fn clicks_wait_for_an_open_prompt() -> Result<()> {
    use crossterm::event::{KeyCode, KeyModifiers};
    use vpdf_terminal::decode::{Button, Mouse, MouseKind, Spot};
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let rows = || {
      vec![Row::new(file.into(), String::new(), Destination::File(file.into()))]
    };
    let screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let click = Event::Mouse(Mouse {
      kind: MouseKind::Press(Button::LEFT),
      at: Spot::Cell { column: 5, row: 1 },
      modifiers: KeyModifiers::NONE,
    });
    for (typed, expected) in [
      (":", Effect::Stay),
      ("/", Effect::Stay),
      ("", Effect::Open(file.into())),
    ] {
      let mut home = Home::new(rows(), &[], screen);
      let mut history = History::default();
      for ch in typed.chars() {
        let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
        home.event(Event::Key(key), &mut history)?;
      }
      assert_eq!(home.event(click.clone(), &mut history)?, expected, "{typed}");
    }
    Ok(())
  }
}
