use crate::app::Window as W;
use crate::motion::Motion;
use crate::motion::Motion as M;
use crate::reader::SelectionKind;
use crate::split::{Axis, Direction as D};
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
  Window(crate::app::Window),
  /// Preview the link under the cursor.
  Peek,
  /// Page thumbnail grid.
  Overview,
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
      "g" | "z" | "i" | "a" | "m" | "'" | "`" | "C-w"
    ) {
      self.clear();
    }
    None
  }

  /// The prefix waiting for its next key, or `"` choosing a register.
  pub fn awaiting(&self) -> Option<&str> {
    if self.choosing_register {
      return Some("\"");
    }
    (!self.prefix.is_empty()).then_some(self.prefix.as_str())
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
    KeyCode::Char(' ') if key.modifiers.contains(KeyModifiers::SHIFT) => {
      "S-Space".into()
    }
    KeyCode::Char(ch) => ch.to_string(),
    KeyCode::Backspace => "BS".into(),
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

/// Keys, action, and description, also shown as prefix-key clues.
const BINDINGS: &[(&str, Action, &str)] = &[
  ("h", Action::Motion(M::Left), "left"),
  ("l", Action::Motion(M::Right), "right"),
  ("j", Action::Motion(M::Down), "down"),
  ("k", Action::Motion(M::Up), "up"),
  ("w", Action::Motion(M::Word), "next word"),
  ("b", Action::Motion(M::BackWord), "previous word"),
  ("e", Action::Motion(M::EndWord), "word end"),
  ("{", Action::Motion(M::PreviousParagraph), "previous paragraph"),
  ("}", Action::Motion(M::NextParagraph), "next paragraph"),
  ("0", Action::Motion(M::LineStart), "line start"),
  ("^", Action::Motion(M::FirstNonblank), "first non-blank"),
  ("$", Action::Motion(M::LineEnd), "line end"),
  ("gg", Action::Motion(M::First), "first page, or [count] page"),
  ("G", Action::Motion(M::Last), "last page, or [count] page"),
  (" ", Action::Motion(M::NextPage), "next page"),
  ("S-Space", Action::Motion(M::PreviousPage), "previous page"),
  ("BS", Action::Motion(M::PreviousPage), "previous page"),
  ("y", Action::Yank, "yank"),
  ("Y", Action::YankLine, "yank line"),
  ("iw", Action::Object(false), "inner word"),
  ("aw", Action::Object(true), "a word"),
  ("v", Action::Visual(SelectionKind::Character), "select characters"),
  ("V", Action::Visual(SelectionKind::Line), "select lines"),
  ("o", Action::Swap, "other selection end"),
  ("gv", Action::Reselect, "reselect"),
  ("q", Action::Quit, "quit"),
  ("C-c", Action::Quit, "quit"),
  ("/", Action::Search(false), "search forward"),
  ("?", Action::Search(true), "search backward"),
  ("*", Action::WordSearch, "search cursor word"),
  ("n", Action::Result(false), "next match"),
  ("N", Action::Result(true), "previous match"),
  ("go", Action::Panel(PanelKind::Outline), "table of contents"),
  ("gs", Action::Panel(PanelKind::Results), "search results"),
  ("gl", Action::Panel(PanelKind::Links), "links on this page"),
  ("gr", Action::Panel(PanelKind::Recent), "recent documents"),
  ("ga", Action::Panel(PanelKind::Annotations), "annotations"),
  ("gx", Action::Panel(PanelKind::Text), "text preview"),
  ("g?", Action::Panel(PanelKind::Help), "key reference"),
  ("gq", Action::ClosePanel, "close panel"),
  ("K", Action::Peek, "preview link target"),
  ("gp", Action::Overview, "page thumbnails"),
  ("f", Action::Hints(false), "link hints"),
  ("F", Action::Hints(true), "link hints, copy URLs"),
  ("gt", Action::Tab(true), "next tab, or [count] tab"),
  ("gT", Action::Tab(false), "previous tab"),
  ("Enter", Action::Follow, "follow link"),
  ("C-o", Action::History(false), "jump back"),
  ("C-i", Action::History(true), "jump forward"),
  (":", Action::Command, "command line"),
  ("C-d", Action::Scroll(0.5), "half page down"),
  ("C-u", Action::Scroll(-0.5), "half page up"),
  ("C-f", Action::Scroll(1.0), "page down"),
  ("C-b", Action::Scroll(-1.0), "page up"),
  ("C-e", Action::Rows(1), "scroll down"),
  ("C-y", Action::Rows(-1), "scroll up"),
  ("zt", Action::Align(0.0), "cursor to top"),
  ("zz", Action::Align(0.5), "cursor to middle"),
  ("zb", Action::Align(1.0), "cursor to bottom"),
  ("+", Action::Zoom(1), "zoom in"),
  ("-", Action::Zoom(-1), "zoom out"),
  ("=", Action::Fit, "fit width"),
  ("zf", Action::FitPage, "fit page"),
  ("zh", Action::Pan(-1), "pan left"),
  ("zl", Action::Pan(1), "pan right"),
  ("zr", Action::Rotate, "rotate clockwise"),
  ("zc", Action::Crop, "crop margins"),
  ("C-ws", Action::Window(W::Split(Axis::Rows)), "split above"),
  ("C-wC-s", Action::Window(W::Split(Axis::Rows)), "split above"),
  ("C-wv", Action::Window(W::Split(Axis::Columns)), "split left"),
  ("C-wC-v", Action::Window(W::Split(Axis::Columns)), "split left"),
  ("C-wh", Action::Window(W::Focus(D::Left)), "pane left"),
  ("C-wC-h", Action::Window(W::Focus(D::Left)), "pane left"),
  ("C-wj", Action::Window(W::Focus(D::Down)), "pane below"),
  ("C-wC-j", Action::Window(W::Focus(D::Down)), "pane below"),
  ("C-wk", Action::Window(W::Focus(D::Up)), "pane above"),
  ("C-wC-k", Action::Window(W::Focus(D::Up)), "pane above"),
  ("C-wl", Action::Window(W::Focus(D::Right)), "pane right"),
  ("C-wC-l", Action::Window(W::Focus(D::Right)), "pane right"),
  ("C-ww", Action::Window(W::Next), "next pane, or [count] pane"),
  ("C-wC-w", Action::Window(W::Next), "next pane, or [count] pane"),
  ("C-wW", Action::Window(W::Previous), "previous pane"),
  ("C-wp", Action::Window(W::Last), "last focused pane"),
  ("C-wc", Action::Window(W::Close), "close pane"),
  ("C-wq", Action::Window(W::Close), "close pane"),
  ("C-wC-q", Action::Window(W::Close), "close pane"),
  ("C-wo", Action::Window(W::Only), "close other panes"),
  ("C-wC-o", Action::Window(W::Only), "close other panes"),
  ("C-w=", Action::Window(W::Equalize), "equal sizes"),
  ("C-w+", Action::Window(W::Resize(Axis::Rows, 1)), "taller"),
  ("C-w-", Action::Window(W::Resize(Axis::Rows, -1)), "shorter"),
  ("C-w>", Action::Window(W::Resize(Axis::Columns, 1)), "wider"),
  ("C-w<", Action::Window(W::Resize(Axis::Columns, -1)), "narrower"),
  ("C-w_", Action::Window(W::Maximize(Axis::Rows)), "tallest"),
  ("C-w|", Action::Window(W::Maximize(Axis::Columns)), "widest"),
  ("C-wH", Action::Window(W::Move(D::Left)), "move pane to far left"),
  ("C-wJ", Action::Window(W::Move(D::Down)), "move pane to bottom"),
  ("C-wK", Action::Window(W::Move(D::Up)), "move pane to top"),
  ("C-wL", Action::Window(W::Move(D::Right)), "move pane to far right"),
  ("C-wx", Action::Window(W::Exchange), "swap with sibling pane"),
  ("C-wT", Action::Window(W::ToTab), "move pane to a new tab"),
];

pub fn binding(key: &str) -> Option<Action> {
  BINDINGS.iter().find(|(keys, ..)| *keys == key).map(|(_, action, _)| *action)
}

/// Keys that may follow `prefix`, with descriptions, like mini.clue.
pub fn clues(prefix: &str) -> Vec<(String, &'static str)> {
  let mut clues: Vec<_> = BINDINGS
    .iter()
    .filter(|(keys, ..)| keys.len() > prefix.len() && keys.starts_with(prefix))
    .map(|(keys, _, about)| (keys[prefix.len()..].to_owned(), *about))
    .collect();
  clues.sort_by(|a, b| a.0.cmp(&b.0));
  clues
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
  fn ctrl_w_takes_a_window_command() {
    let mut keys = Keys::default();
    let ctrl_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
    assert!(keys.feed(ctrl_w).is_none());
    assert_eq!(keys.awaiting(), Some("C-w"));
    let input = keys.feed(key('v')).unwrap();
    assert_eq!(input.action, Action::Window(W::Split(Axis::Columns)));
    assert!(keys.feed(ctrl_w).is_none());
    let ctrl_l = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
    let input = keys.feed(ctrl_l).unwrap();
    assert_eq!(input.action, Action::Window(W::Focus(D::Right)));
    keys.feed(ctrl_w);
    let shifted = KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT);
    let input = keys.feed(shifted).unwrap();
    assert_eq!(input.action, Action::Window(W::Move(D::Right)));
  }

  #[test]
  fn escape_resets_pending_count() {
    let mut keys = Keys::default();
    keys.feed(key('9'));
    keys.feed(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(keys.feed(key('j')).unwrap().count, 1);
    assert!(keys.feed(key('d')).is_none());
    assert_eq!(
      keys.feed(key(' ')).unwrap().action,
      Action::Motion(Motion::NextPage)
    );
    let shifted = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::SHIFT);
    assert_eq!(
      keys.feed(shifted).unwrap().action,
      Action::Motion(Motion::PreviousPage)
    );
  }
}
