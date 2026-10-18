use crate::Result;
use crate::cmdline;
use crate::decode::Event;
use crate::graphics::Graphics;
use crate::keys::{Action, Keys, PanelKind};
use crate::layout::Screen;
use crate::panel::{Destination, Panel, Row, recent_rows};
use crate::prompt::{History, Prompt, PromptEvent, PromptKind};
use crate::recent::Shelf;
use crate::state::Entry;
use crate::statusline;
use crate::terminal::{self, At};
use crate::theme::{self, Color};
use crossterm::event::{KeyEvent, KeyEventKind};
use std::io::Write;
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

/// What the start screen asks the session to do.
#[derive(Debug, Eq, PartialEq)]
pub enum Outcome {
  Stay,
  Perf(Option<bool>),
  Open(PathBuf),
  /// Choose a file with `fzf`.
  Pick,
  Quit,
}

/// Start screen: recent documents and a command line to open files.
pub struct Home {
  pub screen: Screen,
  pub(crate) at: At,
  pub error: Option<String>,
  panel: Panel,
  /// Places thumbnails.
  graphics: Graphics,
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
    let mut panel = Panel::new(PanelKind::Recent, rows);
    panel.shelf = Some(Shelf::new(recent, ""));
    Self {
      screen,
      at: At::default(),
      error: None,
      panel,
      graphics: Graphics::default(),
      prompt: None,
      keys: Keys::default(),
    }
  }

  pub(crate) fn updates(&mut self) -> bool {
    self.panel.shelf.as_mut().is_some_and(Shelf::updates)
  }

  pub fn draw(&mut self, out: &mut impl Write) -> Result<()> {
    write!(out, "{}{}", terminal::frame_start(), theme::RESET)?;
    self.updates();
    self.graphics.begin(out)?;
    let at = self.at;
    for row in 0..self.screen.rows {
      terminal::clear(out, at, row, self.screen.columns)?;
    }
    self.panel.draw(
      out,
      at,
      self.screen.columns,
      self.screen,
      Some(&mut self.graphics),
    )?;
    let row = self.screen.rows - 1;
    if let Some(prompt) = &self.prompt {
      statusline::prompt(out, prompt, self.screen, at, None)?;
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
    self.graphics.end(out)?;
    write!(out, "{}", terminal::frame_end())?;
    out.flush()?;
    Ok(())
  }

  /// Whether a prompt takes typed and pasted text.
  pub fn typing(&self) -> bool {
    self.prompt.is_some()
  }

  /// Deletes the thumbnail before the screen goes.
  pub fn release(&mut self, out: &mut impl Write) -> Result<()> {
    self.graphics.clear(out)
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
        // Terminals without drops paste dropped files' paths.
        let Some(prompt) = &mut self.prompt else {
          return Ok(
            crate::dnd::pdf(&text).map_or(Outcome::Stay, Outcome::Open),
          );
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
          return Ok(Outcome::Stay);
        };
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
      let Some(input) = self.keys.feed(key, crate::keys::Grammar::Other) else {
        return Ok(Outcome::Stay);
      };
      if self.panel.navigate(input, usize::from(self.screen.rows)) {
        return Ok(Outcome::Stay);
      }
      return Ok(match input.action {
        Action::Follow | Action::Swap => self.selected(),
        Action::Command => {
          self.prompt = Some(Prompt::new(PromptKind::Command));
          Outcome::Stay
        }
        Action::Search(backward) => {
          self.panel.begin_search(backward);
          self.prompt = Some(Prompt::new(PromptKind::Search(backward)));
          Outcome::Stay
        }
        Action::Quit => Outcome::Quit,
        _ => Outcome::Stay,
      });
    };
    let event = cmdline::feed(prompt, key, history)?;
    self.prompt_event(event, history)
  }

  fn prompt_event(
    &mut self,
    event: PromptEvent,
    history: &mut History,
  ) -> Result<Outcome> {
    let Some(prompt) = &mut self.prompt else { return Ok(Outcome::Stay) };
    let kind = prompt.kind;
    let search = matches!(kind, PromptKind::Search(_));
    match event {
      PromptEvent::Edited if search => self.panel.preview(&prompt.text),
      PromptEvent::Cancel => {
        self.prompt = None;
        self.panel.abort_search();
      }
      PromptEvent::Submit => {
        let text = prompt.text.clone();
        self.prompt = None;
        history.push(kind, &text);
        if !search {
          return command(&text);
        }
        anyhow::ensure!(
          self.panel.finish_search(&text),
          "Pattern not found: {}",
          self.panel.search.query
        );
      }
      _ => {}
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
  let (name, argument) = cmdline::parse(line);
  if name.is_empty() {
    return Ok(Outcome::Stay);
  }
  let spec = cmdline::lookup(name)
    .ok_or_else(|| anyhow::anyhow!("Unknown command: {name}"))?;
  match spec.name {
    "edit" | "tabedit" | "tabnew" if !argument.is_empty() => {
      Ok(Outcome::Open(cmdline::expand(argument)))
    }
    "edit" | "tabedit" => anyhow::bail!("File required"),
    "quit" | "qall" | "tabclose" => Ok(Outcome::Quit),
    "recent" | "tabnew" => Ok(Outcome::Stay),
    "fzf" => Ok(Outcome::Pick),
    "perf" => crate::perf::switch(argument).map(Outcome::Perf),
    name => anyhow::bail!("Open a document before :{name}"),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

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
    Home::new(&[], screen).draw(&mut out)?;
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
  fn start_commands_open_or_quit() -> Result<()> {
    assert_eq!(command("e a.pdf")?, Outcome::Open("a.pdf".into()));
    assert_eq!(command("tabnew  b.pdf ")?, Outcome::Open("b.pdf".into()));
    assert_eq!(command("q")?, Outcome::Quit);
    assert_eq!(command("")?, Outcome::Stay);
    assert!(command("e").is_err());
    assert!(command("toc").is_err());
    Ok(())
  }

  #[test]
  fn top_stats_offset_home_and_mouse() -> Result<()> {
    use crate::decode::{Button, Mouse, MouseKind, Spot};
    use crossterm::event::KeyModifiers;
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let entry = crate::state::tests::entry(file, 0, 0);
    let screen = Screen {
      columns: 80,
      rows: 20,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 4),
    };
    let mut home = Home::new(&[entry], screen);
    home.at.row = 4;
    let mut out = Vec::new();
    home.draw(&mut out)?;
    let text = String::from_utf8(out)?;
    assert!(text.contains("\x1b[5;1H"));
    assert!(!text.contains("\x1b[1;1H"));
    for (row, expected) in [(1, Outcome::Stay), (5, Outcome::Open(file.into()))]
    {
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
    use crate::decode::{Button, Mouse, MouseKind, Spot};
    use crossterm::event::{KeyCode, KeyModifiers};
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let entry = crate::state::tests::entry(file, 0, 0);
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
      (":", Outcome::Stay),
      ("/", Outcome::Stay),
      ("", Outcome::Open(file.into())),
    ] {
      let mut home = Home::new(std::slice::from_ref(&entry), screen);
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
