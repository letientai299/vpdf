use crate::motion::Motion;
use crate::reader::SelectionKind;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanelKind {
  Outline,
  Results,
  Links,
  Text,
  Help,
  Info,
  Recent,
  Marks,
  Annotations,
}

impl PanelKind {
  pub fn title(self) -> &'static str {
    match self {
      Self::Outline => "CONTENTS",
      Self::Results => "RESULTS",
      Self::Links => "LINKS",
      Self::Text => "TEXT",
      Self::Help => "HELP",
      Self::Info => "INFO",
      Self::Recent => "RECENT",
      Self::Marks => "MARKS",
      Self::Annotations => "ANNOTATIONS",
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
  Motion(Motion),
  Yank,
  YankLine,
  Object(bool),
  Visual(SelectionKind),
  Swap,
  Reselect,
  Cancel,
  Quit,
  Search(bool),
  WordSearch,
  Result(bool),
  Panel(PanelKind),
  ClosePanel,
  Follow,
  History(bool),
  Command,
  Tab(bool),
  /// Link hints; `true` copies URIs instead of opening them.
  Hints(bool),
  /// Preview the link under the cursor.
  Peek,
  Mark(char),
  /// Mark name and whether to keep the glyph column.
  GoMark(char, bool),
  Scroll(f32),
  Rows(i32),
  Align(f32),
  Zoom(i32),
  Fit,
  FitPage,
  Pan(i32),
  Rotate,
  Crop,
}

#[derive(Clone, Copy, Debug)]
pub struct Input {
  pub action: Action,
  pub count: usize,
  pub explicit: bool,
  pub register: char,
}

#[derive(Default)]
pub struct Keys {
  prefix: String,
  count: usize,
  register: Option<char>,
  choosing_register: bool,
}

impl Keys {
  pub fn clear(&mut self) {
    *self = Self::default();
  }

  pub fn feed(&mut self, key: KeyEvent) -> Option<Input> {
    if key.code == KeyCode::Esc {
      self.clear();
      return Some(self.finish(Action::Cancel));
    }
    let token = token(key)?;
    if self.choosing_register {
      self.register = token.chars().next();
      self.choosing_register = false;
      return None;
    }
    if token == "\"" {
      self.choosing_register = true;
      return None;
    }
    if matches!(self.prefix.as_str(), "m" | "'" | "`") {
      let action = mark(&self.prefix, &token);
      return match action {
        Some(action) => Some(self.finish(action)),
        None => {
          self.clear();
          None
        }
      };
    }
    if token.len() == 1
      && let Ok(digit) = token.parse::<usize>()
      && (digit != 0 || self.count != 0)
    {
      self.count =
        self.count.saturating_mul(10).saturating_add(digit).min(10_000);
      return None;
    }
    self.prefix.push_str(&token);
    if let Some(action) = binding(&self.prefix) {
      return Some(self.finish(action));
    }
    if !matches!(
      self.prefix.as_str(),
      "g" | "z" | "[" | "]" | "i" | "a" | "m" | "'" | "`"
    ) {
      self.clear();
    }
    None
  }

  /// Typed count, register, and prefix, like Vim's showcmd.
  pub fn pending(&self) -> String {
    let mut text = String::new();
    if let Some(register) = self.register {
      text.push('"');
      text.push(register);
    } else if self.choosing_register {
      text.push('"');
    }
    if self.count > 0 {
      text += &self.count.to_string();
    }
    text + &self.prefix
  }

  fn finish(&mut self, action: Action) -> Input {
    let input = Input {
      action,
      count: self.count.max(1),
      explicit: self.count > 0,
      register: self.register.unwrap_or('"'),
    };
    self.clear();
    input
  }
}

fn token(key: KeyEvent) -> Option<String> {
  if key.modifiers.contains(KeyModifiers::CONTROL) {
    return match key.code {
      KeyCode::Char(ch) => Some(format!("C-{ch}")),
      _ => None,
    };
  }
  Some(match key.code {
    KeyCode::Char(ch) => ch.to_string(),
    KeyCode::Left => "h".into(),
    KeyCode::Right => "l".into(),
    KeyCode::Down => "j".into(),
    KeyCode::Up => "k".into(),
    KeyCode::PageDown => "C-f".into(),
    KeyCode::PageUp => "C-b".into(),
    KeyCode::Enter => "Enter".into(),
    KeyCode::Tab => "C-i".into(),
    KeyCode::Home => "gg".into(),
    KeyCode::End => "G".into(),
    KeyCode::F(1) => "g?".into(),
    _ => return None,
  })
}

/// `m`, `'`, and `` ` `` take a mark name; `''` returns like Ctrl-o.
fn mark(prefix: &str, token: &str) -> Option<Action> {
  let mut chars = token.chars();
  let name = chars.next().filter(|_| chars.next().is_none())?;
  match (prefix, name) {
    ("'" | "`", '\'' | '`') => Some(Action::History(false)),
    ("m", name) if name.is_ascii_alphabetic() => Some(Action::Mark(name)),
    ("'", name) if name.is_ascii_alphabetic() => {
      Some(Action::GoMark(name, false))
    }
    ("`", name) if name.is_ascii_alphabetic() => {
      Some(Action::GoMark(name, true))
    }
    _ => None,
  }
}

pub fn binding(key: &str) -> Option<Action> {
  use crate::motion::Motion as M;
  use Action::*;
  Some(match key {
    "h" => Motion(M::Left),
    "l" => Motion(M::Right),
    "j" => Motion(M::Down),
    "k" => Motion(M::Up),
    "w" => Motion(M::Word),
    "b" => Motion(M::BackWord),
    "e" => Motion(M::EndWord),
    "{" => Motion(M::PreviousParagraph),
    "}" => Motion(M::NextParagraph),
    "0" => Motion(M::LineStart),
    "^" => Motion(M::FirstNonblank),
    "$" => Motion(M::LineEnd),
    "gg" => Motion(M::First),
    "G" => Motion(M::Last),
    "]p" => Motion(M::NextPage),
    "[p" => Motion(M::PreviousPage),
    "y" => Yank,
    "Y" => YankLine,
    "iw" => Object(false),
    "aw" => Object(true),
    "v" => Visual(SelectionKind::Character),
    "V" => Visual(SelectionKind::Line),
    "o" => Swap,
    "gv" => Reselect,
    "q" | "C-c" => Quit,
    "/" => Search(false),
    "?" => Search(true),
    "*" => WordSearch,
    "n" => Result(false),
    "N" => Result(true),
    "go" => Panel(PanelKind::Outline),
    "gs" => Panel(PanelKind::Results),
    "gl" => Panel(PanelKind::Links),
    "gr" => Panel(PanelKind::Recent),
    "ga" => Panel(PanelKind::Annotations),
    "gx" => Panel(PanelKind::Text),
    "g?" => Panel(PanelKind::Help),
    "gq" => ClosePanel,
    "K" => Peek,
    "f" => Hints(false),
    "F" => Hints(true),
    "gt" => Tab(true),
    "gT" => Tab(false),
    "Enter" => Follow,
    "C-o" => History(false),
    "C-i" => History(true),
    ":" => Command,
    "C-d" => Scroll(0.5),
    "C-u" => Scroll(-0.5),
    "C-f" => Scroll(1.0),
    "C-b" => Scroll(-1.0),
    "C-e" => Rows(1),
    "C-y" => Rows(-1),
    "zt" => Align(0.0),
    "zz" => Align(0.5),
    "zb" => Align(1.0),
    "+" => Zoom(1),
    "-" => Zoom(-1),
    "=" => Fit,
    "zf" => FitPage,
    "zh" => Pan(-1),
    "zl" => Pan(1),
    "zr" => Rotate,
    "zc" => Crop,
    _ => return None,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn key(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
  }

  #[test]
  fn count_register_and_prefix() {
    let mut keys = Keys::default();
    for ch in "\"a12g".chars() {
      assert!(keys.feed(key(ch)).is_none());
    }
    let input = keys.feed(key('g')).unwrap();
    assert_eq!(input.count, 12);
    assert_eq!(input.register, 'a');
    assert_eq!(input.action, Action::Motion(Motion::First));
    assert_eq!(
      keys.feed(key('0')).unwrap().action,
      Action::Motion(Motion::LineStart)
    );
  }

  #[test]
  fn marks_take_a_name() {
    let mut keys = Keys::default();
    let mut feed = |text: &str| {
      text.chars().filter_map(|ch| keys.feed(key(ch))).map(|i| i.action).last()
    };
    assert_eq!(feed("ma"), Some(Action::Mark('a')));
    assert_eq!(feed("'B"), Some(Action::GoMark('B', false)));
    assert_eq!(feed("`a"), Some(Action::GoMark('a', true)));
    assert_eq!(feed("''"), Some(Action::History(false)));
    assert_eq!(feed("m1j"), Some(Action::Motion(Motion::Down)));
  }

  #[test]
  fn escape_resets_pending_count() {
    let mut keys = Keys::default();
    keys.feed(key('9'));
    keys.feed(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(keys.feed(key('j')).unwrap().count, 1);
    assert!(keys.feed(key('d')).is_none());
    assert!(keys.feed(key(' ')).is_none());
  }
}
