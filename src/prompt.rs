//! Command and search line with zsh emacs-keymap editing.
//!
//! Beyond the emacs defaults, it mirrors zsh-autosuggestions (ghost text
//! from history, accepted with Ctrl-Space, Right, or End), Ctrl-w and
//! Ctrl-arrows that stop at punctuation, history search on Ctrl-r, and
//! `edit-command-line` on Ctrl-x Ctrl-e.

use crate::grid::{self, Grid};
use crate::line::{self, NARROW, SHELL};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_width::UnicodeWidthStr;

const LIMIT: usize = 4096;
const HISTORY: usize = 200;
const UNDO: usize = 100;

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

/// Completion candidates replacing `original[start..end]`, listed like
/// zsh; `selected` is set once Tab starts cycling.
pub struct Menu {
  pub items: Vec<String>,
  pub selected: Option<usize>,
  start: usize,
  end: usize,
  original: String,
}

impl Menu {
  /// Paths show their last component.
  pub fn layout(&self, columns: usize, limit: usize) -> Grid {
    let labels = self
      .items
      .iter()
      .map(|item| {
        let start = item.trim_end_matches('/').rfind('/').map_or(0, |i| i + 1);
        item[start..].to_owned()
      })
      .collect();
    grid::layout(labels, columns, limit, self.selected)
  }
}

/// Position in history while browsing, and the line before browsing.
struct Recall {
  index: usize,
  saved: String,
  /// Ctrl-r search text; `None` for plain Up and Down.
  pattern: Option<String>,
}

pub struct Prompt {
  pub kind: PromptKind,
  pub text: String,
  pub menu: Option<Menu>,
  /// History suffix offered after the cursor, like zsh-autosuggestions.
  pub ghost: String,
  cursor: usize,
  recall: Option<Recall>,
  kill: String,
  undo: Vec<(String, usize)>,
  /// Ctrl-x was pressed and waits for its second key.
  prefix: bool,
}

#[derive(Debug, Eq, PartialEq)]
pub enum PromptEvent {
  Pending,
  Edited,
  Complete,
  /// Edit the line in `$EDITOR`.
  Editor,
  Submit,
  Cancel,
}

impl Prompt {
  pub fn new(kind: PromptKind) -> Self {
    Self {
      kind,
      text: String::new(),
      menu: None,
      ghost: String::new(),
      cursor: 0,
      recall: None,
      kill: String::new(),
      undo: Vec::new(),
      prefix: false,
    }
  }

  pub fn before_cursor(&self) -> &str {
    &self.text[..self.cursor]
  }

  /// Text, ghost suggestion, and cursor column as drawn.
  pub fn display(&self) -> (String, &str, usize) {
    let prefix = match self.kind {
      PromptKind::Search(false) => "/",
      PromptKind::Search(true) => "?",
      PromptKind::Command => ":",
      PromptKind::Password => {
        let stars = "*".repeat(self.text.chars().count());
        let column = self.before_cursor().chars().count();
        return (format!("Password: {stars}"), "", 10 + column);
      }
    };
    let column = prefix.width() + self.before_cursor().width();
    (format!("{prefix}{}", self.text), &self.ghost, column)
  }

  /// Replaces the whole line, as after editing it externally.
  pub fn replace(&mut self, text: &str) {
    self.snapshot();
    self.text.clear();
    self.cursor = 0;
    self.insert(&text.replace(['\n', '\r'], " "));
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

  fn snapshot(&mut self) {
    if self.undo.last().is_none_or(|(text, _)| *text != self.text) {
      self.undo.push((self.text.clone(), self.cursor));
      if self.undo.len() > UNDO {
        self.undo.remove(0);
      }
    }
  }

  pub fn feed(&mut self, key: KeyEvent, history: &History) -> PromptEvent {
    let event = self.dispatch(key, history);
    self.suggest(history);
    event
  }

  fn dispatch(&mut self, key: KeyEvent, history: &History) -> PromptEvent {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    if std::mem::take(&mut self.prefix) {
      return match key.code {
        KeyCode::Char('e') => PromptEvent::Editor,
        KeyCode::Char('u') if control => self.restore(),
        _ => PromptEvent::Pending,
      };
    }
    match key.code {
      KeyCode::Tab if self.kind != PromptKind::Password => {
        return self.cycle(true);
      }
      KeyCode::BackTab => return self.cycle(false),
      _ => self.menu = None,
    }
    let browsing = matches!(
      (control, key.code),
      (true, KeyCode::Char('p' | 'n' | 'r' | 's'))
        | (_, KeyCode::Up | KeyCode::Down)
    );
    if !browsing {
      self.recall = None;
    }
    let before = self.text.clone();
    let event = match (control, alt, key.code) {
      (true, _, KeyCode::Char('c' | 'g')) | (_, _, KeyCode::Esc) => {
        return PromptEvent::Cancel;
      }
      (true, _, KeyCode::Char('j' | 'm')) | (_, _, KeyCode::Enter) => {
        return PromptEvent::Submit;
      }
      (true, _, KeyCode::Char('x')) => {
        self.prefix = true;
        return PromptEvent::Pending;
      }
      (true, _, KeyCode::Char('_' | '7' | '/')) => return self.restore(),
      (true, _, KeyCode::Char('d')) if self.cursor == self.text.len() => {
        return self.cycle(true);
      }
      (true, _, KeyCode::Char(ch)) => self.control(ch, history),
      (true, _, KeyCode::Left) => self.word_motion(false),
      (true, _, KeyCode::Right) => self.word_motion(true),
      (_, true, code) => self.meta(code),
      (_, _, code) => self.plain(code, history),
    };
    if event != PromptEvent::Pending {
      return event;
    }
    if self.text == before { PromptEvent::Pending } else { PromptEvent::Edited }
  }

  fn control(&mut self, ch: char, history: &History) -> PromptEvent {
    match ch {
      'a' => self.cursor = 0,
      'e' | 'f' if self.accept() => {}
      'e' => self.cursor = self.text.len(),
      'b' => self.step(-1),
      'f' => self.step(1),
      ' ' => {
        self.accept();
      }
      'h' => self.edit(Self::delete_back),
      'd' => self.edit(|p| p.remove(p.cursor..p.next_char())),
      'k' => self.edit(|p| p.remove(p.cursor..p.text.len())),
      'u' => self.edit(|p| p.remove(0..p.text.len())),
      'w' => self.edit(|p| {
        p.remove(line::word_start(&p.text, p.cursor, NARROW)..p.cursor)
      }),
      'y' => {
        let kill = self.kill.clone();
        self.edit(|p| p.insert(&kill));
      }
      't' => self.edit(Self::transpose),
      'p' => self.recall(history, -1, false),
      'n' => self.recall(history, 1, false),
      'r' => self.recall(history, -1, true),
      's' => self.recall(history, 1, true),
      _ => {}
    }
    PromptEvent::Pending
  }

  /// Alt bindings use zsh's default `WORDCHARS`.
  fn meta(&mut self, code: KeyCode) -> PromptEvent {
    match code {
      KeyCode::Char('b') | KeyCode::Left => {
        self.cursor = line::word_start(&self.text, self.cursor, SHELL);
      }
      KeyCode::Char('f') | KeyCode::Right => self.forward_word(SHELL),
      KeyCode::Char('d') => {
        self.edit(|p| {
          p.remove(p.cursor..line::word_end(&p.text, p.cursor, SHELL))
        });
      }
      KeyCode::Backspace => {
        self.edit(|p| {
          p.remove(line::word_start(&p.text, p.cursor, SHELL)..p.cursor)
        });
      }
      _ => {}
    }
    PromptEvent::Pending
  }

  fn plain(&mut self, code: KeyCode, history: &History) -> PromptEvent {
    match code {
      KeyCode::Backspace => self.edit(Self::delete_back),
      KeyCode::Delete => self.edit(|p| p.remove(p.cursor..p.next_char())),
      KeyCode::Up => self.recall(history, -1, false),
      KeyCode::Down => self.recall(history, 1, false),
      KeyCode::Left => self.step(-1),
      KeyCode::Right | KeyCode::End if self.accept() => {}
      KeyCode::Right => self.step(1),
      KeyCode::Home => self.cursor = 0,
      KeyCode::End => self.cursor = self.text.len(),
      KeyCode::Char(ch) => {
        self.snapshot();
        self.insert(ch.encode_utf8(&mut [0; 4]));
      }
      _ => {}
    }
    PromptEvent::Pending
  }

  /// Ctrl-arrows move by narrow words, as in the zsh config.
  fn word_motion(&mut self, forward: bool) -> PromptEvent {
    if forward {
      self.forward_word(NARROW);
    } else {
      self.cursor = line::word_start(&self.text, self.cursor, NARROW);
    }
    PromptEvent::Pending
  }

  /// At the end of the line, also takes the suggestion's next word.
  fn forward_word(&mut self, chars: &str) {
    if self.cursor == self.text.len() && !self.ghost.is_empty() {
      let end = line::next_word(&self.ghost, 0, chars).max(1);
      let end =
        (1..=end).rev().find(|i| self.ghost.is_char_boundary(*i)).unwrap_or(0);
      let taken = self.ghost[..end].to_owned();
      self.snapshot();
      self.insert(&taken);
      return;
    }
    self.cursor = line::next_word(&self.text, self.cursor, chars);
  }

  fn edit(&mut self, change: impl FnOnce(&mut Self)) {
    self.snapshot();
    change(self);
  }

  /// Deletes `range`, keeping multi-character kills for Ctrl-y.
  fn remove(&mut self, range: std::ops::Range<usize>) {
    if range.is_empty() {
      return;
    }
    let removed: String = self.text.drain(range.clone()).collect();
    if removed.chars().count() > 1 {
      self.kill = removed;
    }
    if self.cursor > range.start {
      self.cursor = range.start.max(self.cursor.saturating_sub(range.len()));
    }
  }

  fn next_char(&self) -> usize {
    self.text[self.cursor..]
      .chars()
      .next()
      .map_or(self.cursor, |ch| self.cursor + ch.len_utf8())
  }

  fn delete_back(&mut self) {
    if let Some(ch) = self.before_cursor().chars().next_back() {
      self.cursor -= ch.len_utf8();
      self.text.remove(self.cursor);
    }
  }

  /// Ctrl-t: swaps the characters around the cursor.
  fn transpose(&mut self) {
    let at = if self.cursor == self.text.len() {
      self
        .before_cursor()
        .chars()
        .next_back()
        .map_or(0, |c| self.cursor - c.len_utf8())
    } else {
      self.cursor
    };
    let Some(before) = self.text[..at].chars().next_back() else { return };
    let Some(after) = self.text[at..].chars().next() else { return };
    let start = at - before.len_utf8();
    let end = at + after.len_utf8();
    self.text.replace_range(start..end, &format!("{after}{before}"));
    self.cursor = end;
  }

  fn restore(&mut self) -> PromptEvent {
    let Some((text, cursor)) = self.undo.pop() else {
      return PromptEvent::Pending;
    };
    self.text = text;
    self.cursor = cursor;
    PromptEvent::Edited
  }

  fn step(&mut self, direction: i32) {
    if direction < 0 {
      if let Some(ch) = self.before_cursor().chars().next_back() {
        self.cursor -= ch.len_utf8();
      }
    } else {
      self.cursor = self.next_char();
    }
  }

  /// Takes the whole suggestion; false when there is none to take.
  fn accept(&mut self) -> bool {
    if self.cursor != self.text.len() || self.ghost.is_empty() {
      return false;
    }
    let ghost = std::mem::take(&mut self.ghost);
    self.snapshot();
    self.insert(&ghost);
    true
  }

  /// Newest history entry extending the line, shown after the cursor.
  fn suggest(&mut self, history: &History) {
    self.ghost.clear();
    if self.text.is_empty()
      || self.cursor != self.text.len()
      || self.recall.is_some()
    {
      return;
    }
    if let Some(entry) = history.entries(self.kind).iter().rev().find(|entry| {
      entry.len() > self.text.len() && entry.starts_with(&self.text)
    }) {
      self.ghost = entry[self.text.len()..].to_owned();
    }
  }

  /// Up and Down walk history like zsh; Ctrl-r and Ctrl-s walk entries
  /// that contain the text typed before searching.
  fn recall(&mut self, history: &History, direction: i32, search: bool) {
    let entries = history.entries(self.kind);
    let recall = self.recall.get_or_insert_with(|| Recall {
      index: entries.len(),
      saved: self.text.clone(),
      pattern: None,
    });
    if search && recall.pattern.is_none() {
      recall.pattern = Some(recall.saved.clone());
    }
    let matches = |entry: &String| {
      recall
        .pattern
        .as_ref()
        .is_none_or(|pattern| entry.contains(pattern.as_str()))
    };
    let found = if direction < 0 {
      entries[..recall.index.min(entries.len())].iter().rposition(matches)
    } else {
      entries
        .iter()
        .skip(recall.index + 1)
        .position(matches)
        .map(|offset| recall.index + 1 + offset)
    };
    match found {
      Some(index) => {
        recall.index = index;
        self.text.clone_from(&entries[index]);
      }
      None if direction > 0 => {
        recall.index = entries.len();
        self.text.clone_from(&recall.saved);
      }
      None => {}
    }
    self.cursor = self.text.len();
  }

  /// First Tab lists matches and inserts their common prefix, like zsh;
  /// later Tabs cycle through them.
  fn cycle(&mut self, forward: bool) -> PromptEvent {
    let Some(menu) = &mut self.menu else {
      return if forward {
        PromptEvent::Complete
      } else {
        PromptEvent::Pending
      };
    };
    let count = menu.items.len();
    menu.selected = Some(match menu.selected {
      None if forward => 0,
      None => count - 1,
      Some(index) if forward => (index + 1) % count,
      Some(index) => (index + count - 1) % count,
    });
    self.apply();
    PromptEvent::Edited
  }

  /// Offers `items` for the text from byte `start` to the cursor.
  pub fn complete(&mut self, start: usize, items: Vec<String>) {
    if items.is_empty() || start > self.cursor {
      return;
    }
    self.snapshot();
    let common = common_prefix(&items);
    let single = items.len() == 1;
    self.menu = Some(Menu {
      items,
      selected: None,
      start,
      end: self.cursor,
      original: self.text.clone(),
    });
    if single {
      if let Some(menu) = &mut self.menu {
        menu.selected = Some(0);
      }
      self.apply();
      self.menu = None;
    } else if common.len() > self.cursor - start {
      self.text.replace_range(start..self.cursor, &common);
      self.cursor = start + common.len();
    }
  }

  fn apply(&mut self) {
    let Some(menu) = &self.menu else { return };
    let Some(item) = menu.selected.map(|index| &menu.items[index]) else {
      return;
    };
    self.text = format!(
      "{}{item}{}",
      &menu.original[..menu.start],
      &menu.original[menu.end..]
    );
    self.cursor = menu.start + item.len();
  }
}

fn common_prefix(items: &[String]) -> String {
  let Some(first) = items.first() else { return String::new() };
  let mut end = first.len();
  for item in &items[1..] {
    end = first
      .char_indices()
      .zip(item.chars())
      .find(|((_, a), b)| a != b)
      .map_or(end.min(item.len()), |((index, _), _)| index)
      .min(end);
  }
  first[..end].to_owned()
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
      crate::terminal::At::default(),
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

  fn press(
    prompt: &mut Prompt,
    history: &History,
    keys: &[(KeyCode, KeyModifiers)],
  ) {
    for (code, modifiers) in keys {
      prompt.feed(KeyEvent::new(*code, *modifiers), history);
    }
  }

  const NONE: KeyModifiers = KeyModifiers::NONE;
  const CTRL: KeyModifiers = KeyModifiers::CONTROL;
  const ALT: KeyModifiers = KeyModifiers::ALT;

  fn typed(text: &str, history: &History) -> Prompt {
    let mut prompt = Prompt::new(PromptKind::Command);
    for ch in text.chars() {
      prompt.feed(KeyEvent::new(KeyCode::Char(ch), NONE), history);
    }
    prompt
  }

  #[test]
  fn emacs_editing_keys() {
    let history = History::default();
    let mut prompt = typed("e ~/papers/attention.pdf", &history);
    press(&mut prompt, &history, &[(KeyCode::Char('w'), CTRL)]);
    assert_eq!(prompt.text, "e ~/papers/attention.");
    press(&mut prompt, &history, &[(KeyCode::Backspace, ALT)]);
    assert_eq!(prompt.text, "e ");
    press(&mut prompt, &history, &[(KeyCode::Char('y'), CTRL)]);
    assert_eq!(prompt.text, "e ~/papers/attention.");
    press(&mut prompt, &history, &[(KeyCode::Char('_'), CTRL)]);
    assert_eq!(prompt.text, "e ");
    press(&mut prompt, &history, &[(KeyCode::Char('u'), CTRL)]);
    assert_eq!(prompt.text, "");
    let mut prompt = typed("pgae", &history);
    let back = (KeyCode::Char('b'), CTRL);
    press(&mut prompt, &history, &[back, back, (KeyCode::Char('t'), CTRL)]);
    assert_eq!(prompt.text, "page");
    press(
      &mut prompt,
      &history,
      &[(KeyCode::Char('a'), CTRL), (KeyCode::Char('k'), CTRL)],
    );
    assert_eq!((prompt.text.as_str(), prompt.display().2), ("", 1));
  }

  #[test]
  fn suggestions_come_from_history() {
    let mut history = History::default();
    for entry in ["ocr deu", "page 4", "ocr eng"] {
      history.push(PromptKind::Command, entry);
    }
    let mut prompt = typed("oc", &history);
    assert_eq!(prompt.ghost, "r eng");
    press(&mut prompt, &history, &[(KeyCode::Char('f'), ALT)]);
    assert_eq!(prompt.text, "ocr ");
    press(&mut prompt, &history, &[(KeyCode::Char(' '), CTRL)]);
    assert_eq!((prompt.text.as_str(), prompt.ghost.as_str()), ("ocr eng", ""));
  }

  #[test]
  fn history_walks_like_zsh() {
    let mut history = History::default();
    for entry in ["ocr deu", "page 4", "ocr eng", "page 4"] {
      history.push(PromptKind::Command, entry);
    }
    assert_eq!(history.commands, ["ocr deu", "ocr eng", "page 4"]);
    let mut prompt = typed("x", &history);
    press(&mut prompt, &history, &[(KeyCode::Up, NONE)]);
    assert_eq!(prompt.text, "page 4");
    press(&mut prompt, &history, &[(KeyCode::Down, NONE)]);
    assert_eq!(prompt.text, "x");
    let mut prompt = typed("deu", &history);
    press(&mut prompt, &history, &[(KeyCode::Char('r'), CTRL)]);
    assert_eq!(prompt.text, "ocr deu");
  }

  #[test]
  fn completion_lists_then_cycles() {
    let history = History::default();
    let mut prompt = typed("t", &history);
    let tab = KeyEvent::new(KeyCode::Tab, NONE);
    assert_eq!(prompt.feed(tab, &history), PromptEvent::Complete);
    prompt.complete(0, vec!["tabedit".into(), "tabnext".into(), "text".into()]);
    assert_eq!(prompt.text, "t");
    prompt.complete(0, vec!["tabedit".into(), "tabnext".into()]);
    assert_eq!(prompt.text, "tab");
    assert_eq!(prompt.menu.as_ref().map(|m| m.selected), Some(None));
    press(&mut prompt, &history, &[(KeyCode::Tab, NONE), (KeyCode::Tab, NONE)]);
    assert_eq!(prompt.text, "tabnext");
    press(
      &mut prompt,
      &history,
      &[(KeyCode::BackTab, NONE), (KeyCode::Char(' '), NONE)],
    );
    assert_eq!(prompt.text, "tabedit ");
    assert!(prompt.menu.is_none());
  }

  #[test]
  fn ctrl_x_ctrl_e_opens_the_editor() {
    let history = History::default();
    let mut prompt = typed("page", &history);
    prompt.feed(KeyEvent::new(KeyCode::Char('x'), CTRL), &history);
    let event = prompt.feed(KeyEvent::new(KeyCode::Char('e'), CTRL), &history);
    assert_eq!(event, PromptEvent::Editor);
  }
}
