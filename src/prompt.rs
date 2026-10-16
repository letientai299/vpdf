use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthStr;

const LIMIT: usize = 4096;
const HISTORY: usize = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptKind {
  Search(bool),
  Command,
  Password,
}

#[derive(Default)]
pub struct History {
  pub commands: Vec<String>,
  pub searches: Vec<String>,
}

impl History {
  fn entries(&self, kind: PromptKind) -> &[String] {
    match kind {
      PromptKind::Search(_) => &self.searches,
      PromptKind::Command => &self.commands,
      PromptKind::Password => &[],
    }
  }

  pub fn push(&mut self, kind: PromptKind, text: &str) {
    let entries = match kind {
      PromptKind::Search(_) => &mut self.searches,
      PromptKind::Command => &mut self.commands,
      PromptKind::Password => return,
    };
    if text.trim().is_empty() {
      return;
    }
    entries.retain(|entry| entry != text);
    entries.push(text.to_owned());
    if entries.len() > HISTORY {
      entries.remove(0);
    }
  }
}

/// Completion candidates replacing `original[start..end]`.
pub struct Menu {
  pub items: Vec<String>,
  pub selected: usize,
  start: usize,
  end: usize,
  original: String,
}

struct Recall {
  index: usize,
  prefix: String,
}

pub struct Prompt {
  pub kind: PromptKind,
  pub text: String,
  pub menu: Option<Menu>,
  cursor: usize,
  recall: Option<Recall>,
}

#[derive(Debug, Eq, PartialEq)]
pub enum PromptEvent {
  Pending,
  Edited,
  Complete,
  Submit,
  Cancel,
}

impl Prompt {
  pub fn new(kind: PromptKind) -> Self {
    Self { kind, text: String::new(), menu: None, cursor: 0, recall: None }
  }

  pub fn before_cursor(&self) -> &str {
    &self.text[..self.cursor]
  }

  /// Text and cursor column as drawn.
  pub fn display(&self) -> (String, usize) {
    let (prefix, body, column) = match self.kind {
      PromptKind::Search(false) => ("/", self.text.clone(), self.text_column()),
      PromptKind::Search(true) => ("?", self.text.clone(), self.text_column()),
      PromptKind::Command => (":", self.text.clone(), self.text_column()),
      PromptKind::Password => (
        "Password: ",
        "*".repeat(self.text.chars().count()),
        self.before_cursor().chars().count(),
      ),
    };
    (format!("{prefix}{body}"), prefix.width() + column)
  }

  fn text_column(&self) -> usize {
    self.before_cursor().width()
  }

  pub fn insert(&mut self, text: &str) {
    for ch in text.chars().filter(|ch| !ch.is_control()) {
      if self.text.len() + ch.len_utf8() > LIMIT {
        break;
      }
      self.text.insert(self.cursor, ch);
      self.cursor += ch.len_utf8();
    }
  }

  pub fn feed(&mut self, key: KeyEvent, history: &History) -> PromptEvent {
    match key.code {
      KeyCode::Tab if self.kind != PromptKind::Password => {
        return self.cycle(true);
      }
      KeyCode::BackTab => return self.cycle(false),
      _ => self.menu = None,
    }
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let recalling = matches!(
      (control, key.code),
      (true, KeyCode::Char('p' | 'n')) | (_, KeyCode::Up | KeyCode::Down)
    );
    if !recalling {
      self.recall = None;
    }
    let before = self.text.clone();
    match (control, key.code) {
      (true, KeyCode::Char('c')) | (_, KeyCode::Esc) => {
        return PromptEvent::Cancel;
      }
      (true, KeyCode::Char('j' | 'm')) | (_, KeyCode::Enter) => {
        return PromptEvent::Submit;
      }
      (true, KeyCode::Char('h')) | (_, KeyCode::Backspace) => {
        if self.text.is_empty() {
          return PromptEvent::Cancel;
        }
        self.delete_back();
      }
      (true, KeyCode::Char('p')) | (_, KeyCode::Up) => self.recall(history, -1),
      (true, KeyCode::Char('n')) | (_, KeyCode::Down) => {
        self.recall(history, 1)
      }
      (true, KeyCode::Char(ch)) => self.control(ch),
      (_, KeyCode::Char(ch)) if !key.modifiers.contains(KeyModifiers::ALT) => {
        self.insert(ch.encode_utf8(&mut [0; 4]));
      }
      (_, KeyCode::Delete) => {
        if let Some(ch) = self.text[self.cursor..].chars().next() {
          self.text.drain(self.cursor..self.cursor + ch.len_utf8());
        }
      }
      (_, KeyCode::Left) => self.step(-1),
      (_, KeyCode::Right) => self.step(1),
      (_, KeyCode::Home) => self.cursor = 0,
      (_, KeyCode::End) => self.cursor = self.text.len(),
      _ => {}
    }
    if self.text == before { PromptEvent::Pending } else { PromptEvent::Edited }
  }

  fn control(&mut self, ch: char) {
    match ch {
      'a' => self.cursor = 0,
      'e' => self.cursor = self.text.len(),
      'b' => self.step(-1),
      'f' => self.step(1),
      'k' => self.text.truncate(self.cursor),
      'u' => {
        self.text.drain(..self.cursor);
        self.cursor = 0;
      }
      'w' => {
        let head = self.before_cursor().trim_end_matches(' ');
        let start = head.rfind(' ').map_or(0, |space| space + 1);
        self.text.drain(start..self.cursor);
        self.cursor = start;
      }
      _ => {}
    }
  }

  fn step(&mut self, direction: i32) {
    if direction < 0 {
      if let Some(ch) = self.before_cursor().chars().next_back() {
        self.cursor -= ch.len_utf8();
      }
    } else if let Some(ch) = self.text[self.cursor..].chars().next() {
      self.cursor += ch.len_utf8();
    }
  }

  fn delete_back(&mut self) {
    if let Some(ch) = self.before_cursor().chars().next_back() {
      self.cursor -= ch.len_utf8();
      self.text.remove(self.cursor);
    }
  }

  /// Steps through history entries that start with the typed prefix.
  fn recall(&mut self, history: &History, direction: i32) {
    let entries = history.entries(self.kind);
    let recall = self.recall.get_or_insert_with(|| Recall {
      index: entries.len(),
      prefix: self.text.clone(),
    });
    let matches = |entry: &&String| entry.starts_with(&recall.prefix);
    let found = if direction < 0 {
      entries[..recall.index.min(entries.len())]
        .iter()
        .rposition(|entry| matches(&entry))
    } else {
      entries
        .iter()
        .skip(recall.index + 1)
        .position(|entry| matches(&entry))
        .map(|offset| recall.index + 1 + offset)
    };
    match found {
      Some(index) => {
        recall.index = index;
        self.text.clone_from(&entries[index]);
      }
      None if direction > 0 => {
        recall.index = entries.len();
        self.text.clone_from(&recall.prefix);
      }
      None => {}
    }
    self.cursor = self.text.len();
  }

  fn cycle(&mut self, forward: bool) -> PromptEvent {
    let Some(menu) = &mut self.menu else {
      return if forward {
        PromptEvent::Complete
      } else {
        PromptEvent::Pending
      };
    };
    let count = menu.items.len();
    menu.selected =
      (menu.selected + if forward { 1 } else { count - 1 }) % count;
    self.apply();
    PromptEvent::Edited
  }

  /// Offers `items` for the text from byte `start` to the cursor.
  pub fn complete(&mut self, start: usize, items: Vec<String>) {
    if items.is_empty() || start > self.cursor {
      return;
    }
    let single = items.len() == 1;
    self.menu = Some(Menu {
      items,
      selected: 0,
      start,
      end: self.cursor,
      original: self.text.clone(),
    });
    self.apply();
    if single {
      self.menu = None;
    }
  }

  fn apply(&mut self) {
    let Some(menu) = &self.menu else { return };
    let item = &menu.items[menu.selected];
    self.text = format!(
      "{}{item}{}",
      &menu.original[..menu.start],
      &menu.original[menu.end..]
    );
    self.cursor = menu.start + item.len();
  }
}

pub fn password() -> crate::Result<String> {
  use crossterm::event::{self, Event, KeyEventKind};
  use std::io::{self, Write};
  let mut prompt = Prompt::new(PromptKind::Password);
  let history = History::default();
  let mut out = io::stdout();
  loop {
    let screen = crate::terminal::size()?;
    crate::terminal::line(
      &mut out,
      screen.rows - 1,
      &prompt.display().0,
      screen.columns,
    )?;
    out.flush()?;
    if let Event::Key(key) = event::read()?
      && key.kind != KeyEventKind::Release
    {
      match prompt.feed(key, &history) {
        PromptEvent::Submit => return Ok(prompt.text),
        PromptEvent::Cancel => anyhow::bail!("Password cancelled"),
        _ => {}
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn feed(prompt: &mut Prompt, history: &History, keys: &[KeyCode]) {
    for code in keys {
      prompt.feed(KeyEvent::new(*code, KeyModifiers::NONE), history);
    }
  }

  #[test]
  fn edits_at_cursor() {
    let mut prompt = Prompt::new(PromptKind::Command);
    let history = History::default();
    prompt.insert("pge 3");
    feed(&mut prompt, &history, &[KeyCode::Home, KeyCode::Right]);
    prompt.insert("a");
    assert_eq!(prompt.text, "page 3");
    assert_eq!(prompt.display(), (":page 3".into(), 3));
    feed(&mut prompt, &history, &[KeyCode::End, KeyCode::Backspace]);
    assert_eq!(prompt.text, "page ");
  }

  #[test]
  fn recalls_history_by_prefix() {
    let mut history = History::default();
    for entry in ["ocr deu", "page 4", "ocr eng", "page 4"] {
      history.push(PromptKind::Command, entry);
    }
    assert_eq!(history.commands, ["ocr deu", "ocr eng", "page 4"]);
    let mut prompt = Prompt::new(PromptKind::Command);
    prompt.insert("ocr");
    feed(&mut prompt, &history, &[KeyCode::Up]);
    assert_eq!(prompt.text, "ocr eng");
    feed(&mut prompt, &history, &[KeyCode::Up, KeyCode::Up]);
    assert_eq!(prompt.text, "ocr deu");
    feed(&mut prompt, &history, &[KeyCode::Down, KeyCode::Down]);
    assert_eq!(prompt.text, "ocr");
  }

  #[test]
  fn completion_cycles_and_accepts() {
    let mut prompt = Prompt::new(PromptKind::Command);
    let history = History::default();
    prompt.insert("re");
    assert_eq!(
      prompt.feed(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &history),
      PromptEvent::Complete
    );
    prompt.complete(0, vec!["recent".into(), "reload".into()]);
    assert_eq!(prompt.text, "recent");
    feed(&mut prompt, &history, &[KeyCode::Tab]);
    assert_eq!(prompt.text, "reload");
    feed(&mut prompt, &history, &[KeyCode::BackTab, KeyCode::Char(' ')]);
    assert_eq!(prompt.text, "recent ");
    assert!(prompt.menu.is_none());
  }
}
