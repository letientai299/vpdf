use crate::app::Window as W;
use crate::motion::Motion;
use crate::motion::Motion as M;
use crate::objects::Object as O;
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
  Jumps,
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
      Self::Jumps => "JUMPS",
      Self::Annotations => "ANNOTATIONS",
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
  Motion(Motion),
  Section(bool),
  Figure(bool),
  FigureFit,
  Region,
  Yank,
  YankLine,
  /// `yp`: the document's full path.
  YankPath,
  /// `yY`: lines as laid out, without joining.
  YankRaw,
  Object(crate::objects::Object, bool),
  Visual(SelectionKind),
  Cursor,
  Swap,
  Reselect,
  Cancel,
  Quit,
  Search(bool),
  WordSearch,
  Result(bool),
  Panel(PanelKind),
  ClosePanel,
  LocalFocus,
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

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Grammar {
  Normal,
  /// Cursor mode in the content, without an operator.
  Cursor,
  Objects,
  Other,
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

  pub fn feed(&mut self, key: KeyEvent, grammar: Grammar) -> Option<Input> {
    let key = normalize(key);
    if key.code == KeyCode::Esc {
      self.clear();
      return Some(self.finish(Action::Cancel));
    }
    if std::mem::take(&mut self.choosing_register) {
      match key.code {
        KeyCode::Char(name) if plain(key) => self.register = Some(name),
        _ => self.clear(),
      }
      return None;
    }
    let Some(token) = token(key) else {
      self.clear();
      return None;
    };
    if self.prefix == "y" {
      self.prefix.clear();
      match token.as_str() {
        "p" => return Some(self.finish(Action::YankPath)),
        "Y" => return Some(self.finish(Action::YankRaw)),
        _ => {}
      }
    }
    if token == "\"" && self.prefix.is_empty() {
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
    if self.prefix.is_empty()
      && matches!(token.as_str(), "i" | "a")
      && grammar != Grammar::Objects
    {
      if token == "i" && grammar == Grammar::Normal {
        return Some(self.finish(Action::Cursor));
      }
      self.clear();
      return None;
    }
    self.prefix.push_str(&token);
    if let Some(action) = binding(&self.prefix) {
      let register = self.register;
      let input = self.finish(action);
      // `y` starts its operator now and still takes `p` or `Y`.
      if action == Action::Yank
        && matches!(grammar, Grammar::Normal | Grammar::Cursor)
      {
        self.prefix.push('y');
        self.register = register;
      }
      return Some(input);
    }
    if !matches!(
      self.prefix.as_str(),
      "g" | "z" | "i" | "a" | "m" | "'" | "`" | "C-w" | "[" | "]"
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

/// Maps protocol chords to their legacy keys. Ctrl-i stays apart from
/// Tab, which Kitty's keyboard protocol tells apart.
pub fn normalize(mut key: KeyEvent) -> KeyEvent {
  if !key.modifiers.contains(KeyModifiers::CONTROL) {
    return key;
  }
  // Alternate keys report Ctrl-Shift-v as Ctrl-V; legacy bytes and the
  // bindings know it as Ctrl-v.
  if let KeyCode::Char(ch) = key.code
    && ch.is_ascii_uppercase()
  {
    key.code = KeyCode::Char(ch.to_ascii_lowercase());
    key.modifiers |= KeyModifiers::SHIFT;
  }
  let code = match key.code {
    KeyCode::Char('[') => KeyCode::Esc,
    KeyCode::Char('m') => KeyCode::Enter,
    _ => return key,
  };
  KeyEvent { code, modifiers: key.modifiers - KeyModifiers::CONTROL, ..key }
}

/// No modifier but Shift, which uppercase carries.
pub fn plain(key: KeyEvent) -> bool {
  (key.modifiers - KeyModifiers::SHIFT).is_empty()
}

/// Binding name; Alt and Super chords have none.
fn token(key: KeyEvent) -> Option<String> {
  if key.modifiers - KeyModifiers::SHIFT == KeyModifiers::CONTROL {
    return match key.code {
      KeyCode::Char(ch) => Some(format!("C-{ch}")),
      _ => None,
    };
  }
  if !plain(key) {
    return None;
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
    KeyCode::Tab | KeyCode::BackTab => "Tab".into(),
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
  ("W", Action::Motion(M::BigWord), "next WORD"),
  ("B", Action::Motion(M::BackBigWord), "previous WORD"),
  ("E", Action::Motion(M::EndBigWord), "WORD end"),
  ("gE", Action::Motion(M::PreviousBigEnd), "previous WORD end"),
  ("(", Action::Motion(M::PreviousSentence), "previous sentence"),
  (")", Action::Motion(M::NextSentence), "next sentence"),
  ("[[", Action::Section(false), "previous section"),
  ("]]", Action::Section(true), "next section"),
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
  ("yp", Action::YankPath, "yank file path"),
  ("yY", Action::YankRaw, "yank lines as laid out"),
  ("iw", Action::Object(O::Word, false), "inner word"),
  ("iW", Action::Object(O::BigWord, false), "inner WORD"),
  ("is", Action::Object(O::Sentence, false), "inner sentence"),
  ("ip", Action::Object(O::Paragraph, false), "inner paragraph"),
  ("i(", Action::Object(O::Parentheses, false), "inner parentheses"),
  ("i)", Action::Object(O::Parentheses, false), "inner parentheses"),
  ("ib", Action::Object(O::Parentheses, false), "inner parentheses"),
  ("i[", Action::Object(O::Brackets, false), "inner brackets"),
  ("i]", Action::Object(O::Brackets, false), "inner brackets"),
  ("i{", Action::Object(O::Braces, false), "inner braces"),
  ("i}", Action::Object(O::Braces, false), "inner braces"),
  ("iB", Action::Object(O::Braces, false), "inner braces"),
  ("i<", Action::Object(O::Angles, false), "inner angles"),
  ("i>", Action::Object(O::Angles, false), "inner angles"),
  ("i\"", Action::Object(O::DoubleQuote, false), "inner double quotes"),
  ("i'", Action::Object(O::SingleQuote, false), "inner single quotes"),
  ("i“", Action::Object(O::DoubleQuote, false), "inner double quotes"),
  ("i”", Action::Object(O::DoubleQuote, false), "inner double quotes"),
  ("i‘", Action::Object(O::SingleQuote, false), "inner single quotes"),
  ("i’", Action::Object(O::SingleQuote, false), "inner single quotes"),
  ("aw", Action::Object(O::Word, true), "around word"),
  ("aW", Action::Object(O::BigWord, true), "around WORD"),
  ("as", Action::Object(O::Sentence, true), "around sentence"),
  ("ap", Action::Object(O::Paragraph, true), "around paragraph"),
  ("a(", Action::Object(O::Parentheses, true), "around parentheses"),
  ("a)", Action::Object(O::Parentheses, true), "around parentheses"),
  ("ab", Action::Object(O::Parentheses, true), "around parentheses"),
  ("a[", Action::Object(O::Brackets, true), "around brackets"),
  ("a]", Action::Object(O::Brackets, true), "around brackets"),
  ("a{", Action::Object(O::Braces, true), "around braces"),
  ("a}", Action::Object(O::Braces, true), "around braces"),
  ("aB", Action::Object(O::Braces, true), "around braces"),
  ("a<", Action::Object(O::Angles, true), "around angles"),
  ("a>", Action::Object(O::Angles, true), "around angles"),
  ("a\"", Action::Object(O::DoubleQuote, true), "around double quotes"),
  ("a'", Action::Object(O::SingleQuote, true), "around single quotes"),
  ("a“", Action::Object(O::DoubleQuote, true), "around double quotes"),
  ("a”", Action::Object(O::DoubleQuote, true), "around double quotes"),
  ("a‘", Action::Object(O::SingleQuote, true), "around single quotes"),
  ("a’", Action::Object(O::SingleQuote, true), "around single quotes"),
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
  ("]i", Action::Figure(true), "next figure"),
  ("[i", Action::Figure(false), "previous figure"),
  ("zF", Action::FigureFit, "fit figure"),
  ("gF", Action::Region, "figure actions"),
  ("f", Action::Hints(false), "link hints"),
  ("F", Action::Hints(true), "link hints, copy URLs"),
  ("gt", Action::Tab(true), "next tab, or [count] tab"),
  ("gT", Action::Tab(false), "previous tab"),
  ("Enter", Action::Follow, "follow link"),
  ("C-o", Action::History(false), "jump back"),
  ("C-i", Action::History(true), "jump forward"),
  ("Tab", Action::LocalFocus, "toggle content/panel focus"),
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
  ("C-wr", Action::Window(W::Rotate(true)), "rotate panes forward"),
  ("C-wC-r", Action::Window(W::Rotate(true)), "rotate panes forward"),
  ("C-wR", Action::Window(W::Rotate(false)), "rotate panes backward"),
  ("C-wn", Action::Window(W::New), "new pane with recent documents"),
  ("C-wC-n", Action::Window(W::New), "new pane with recent documents"),
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
      assert!(keys.feed(key(ch), Grammar::Objects).is_none());
    }
    let input = keys.feed(key('g'), Grammar::Objects).unwrap();
    assert_eq!(input.count, 12);
    assert_eq!(input.register, 'a');
    assert_eq!(input.action, Action::Motion(Motion::First));
    assert_eq!(
      keys.feed(key('0'), Grammar::Objects).unwrap().action,
      Action::Motion(Motion::LineStart)
    );
  }

  #[test]
  fn section_prefix_counts() {
    for (text, forward) in [("[[", false), ("]]", true)] {
      let mut keys = Keys::default();
      let mut inputs =
        text.chars().filter_map(|ch| keys.feed(key(ch), Grammar::Objects));
      let input = inputs.next().unwrap();
      assert_eq!(input.action, Action::Section(forward));
      assert_eq!(input.count, 1);
      assert!(!input.explicit);
    }
    for (prefix, forward) in [('[', false), (']', true)] {
      let mut keys = Keys::default();
      keys.feed(key('2'), Grammar::Objects);
      assert!(keys.feed(key(prefix), Grammar::Objects).is_none());
      assert_eq!(keys.pending(), format!("2{prefix}"));
      let labels: Vec<_> =
        clues(&prefix.to_string()).into_iter().map(|(key, _)| key).collect();
      assert!(labels.contains(&prefix.to_string()));
      assert!(labels.contains(&"i".into()));
      assert_eq!(labels.len(), 2);
      let input = keys.feed(key(prefix), Grammar::Objects).unwrap();
      assert_eq!(input.action, Action::Section(forward));
      assert_eq!(input.count, 2);
      assert!(input.explicit);
      assert_eq!(keys.pending(), "");
      keys.feed(key(prefix), Grammar::Objects);
      keys.feed(key('x'), Grammar::Objects);
      assert!(keys.awaiting().is_none());
      assert_eq!(keys.feed(key('j'), Grammar::Objects).unwrap().count, 1);
    }
  }

  #[test]
  fn marks_take_a_name() {
    let mut keys = Keys::default();
    let mut feed = |text: &str| {
      text
        .chars()
        .filter_map(|ch| keys.feed(key(ch), Grammar::Objects))
        .map(|i| i.action)
        .last()
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
    assert!(keys.feed(ctrl_w, Grammar::Objects).is_none());
    assert_eq!(keys.awaiting(), Some("C-w"));
    let input = keys.feed(key('v'), Grammar::Objects).unwrap();
    assert_eq!(input.action, Action::Window(W::Split(Axis::Columns)));
    assert!(keys.feed(ctrl_w, Grammar::Objects).is_none());
    let ctrl_l = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
    let input = keys.feed(ctrl_l, Grammar::Objects).unwrap();
    assert_eq!(input.action, Action::Window(W::Focus(D::Right)));
    keys.feed(ctrl_w, Grammar::Objects);
    let shifted = KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT);
    let input = keys.feed(shifted, Grammar::Objects).unwrap();
    assert_eq!(input.action, Action::Window(W::Move(D::Right)));
  }

  #[test]
  fn normalize_matches_legacy_bytes() {
    let (none, ctrl, alt, shift) = (
      KeyModifiers::NONE,
      KeyModifiers::CONTROL,
      KeyModifiers::ALT,
      KeyModifiers::SHIFT,
    );
    for (code, modifiers, want, left) in [
      (KeyCode::Char('['), ctrl, KeyCode::Esc, none),
      (KeyCode::Char('m'), ctrl, KeyCode::Enter, none),
      (KeyCode::Char('i'), ctrl, KeyCode::Char('i'), ctrl),
      (KeyCode::Tab, none, KeyCode::Tab, none),
      (KeyCode::Char('V'), ctrl, KeyCode::Char('v'), ctrl | shift),
      (KeyCode::Char('M'), ctrl, KeyCode::Enter, shift),
      (KeyCode::Char('['), ctrl | alt, KeyCode::Esc, alt),
      (KeyCode::Char('m'), ctrl | shift, KeyCode::Enter, shift),
      (KeyCode::Char('['), none, KeyCode::Char('['), none),
      (KeyCode::Char('j'), ctrl, KeyCode::Char('j'), ctrl),
    ] {
      let key = normalize(KeyEvent::new(code, modifiers));
      assert_eq!((key.code, key.modifiers), (want, left), "{code:?}");
    }
  }

  #[test]
  fn chords_and_register_names() {
    let with = |ch, modifiers| KeyEvent::new(KeyCode::Char(ch), modifiers);
    let ctrl = |ch| with(ch, KeyModifiers::CONTROL);
    let alt = |ch| with(ch, KeyModifiers::ALT);
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    let f5 = KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE);
    let ctrl_left = KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL);
    let ctrl_shift_v = with('v', KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    let shift_a = with('A', KeyModifiers::SHIFT);
    let out = |action, register| Some((action, 1, register));
    let down = out(Action::Motion(Motion::Down), '"');
    let yank = out(Action::Yank, '"');
    let append = out(Action::Yank, 'A');
    let cancel = out(Action::Cancel, '"');
    let split = out(Action::Window(W::Split(Axis::Columns)), '"');
    let right = out(Action::Motion(Motion::Right), '"');
    let cases = [
      (vec![key('5'), ctrl('['), key('j')], vec![None, cancel, down]),
      (vec![ctrl('m')], vec![out(Action::Follow, '"')]),
      (vec![ctrl('i')], vec![out(Action::History(true), '"')]),
      (vec![alt('q')], vec![None]),
      (vec![with('c', KeyModifiers::CONTROL | KeyModifiers::ALT)], vec![None]),
      (vec![key('5'), alt('j'), key('j')], vec![None, None, down]),
      (vec![key('5'), key('"'), enter, key('j')], vec![None, None, None, down]),
      (vec![key('g'), alt('x'), key('g')], vec![None, None, None]),
      (
        vec![key('"'), key('a'), alt('x'), key('y')],
        vec![None, None, None, yank],
      ),
      (vec![ctrl('w'), ctrl_left, key('l')], vec![None, None, right]),
      (vec![ctrl('w'), ctrl_shift_v], vec![None, split]),
      (vec![key('"'), enter, key('y')], vec![None, None, yank]),
      (vec![key('"'), f5, key('y')], vec![None, None, yank]),
      (vec![key('"'), ctrl('a'), key('y')], vec![None, None, yank]),
      (vec![key('"'), shift_a, key('y')], vec![None, None, append]),
    ];
    for (events, want) in cases {
      let mut keys = Keys::default();
      let got: Vec<_> = events
        .iter()
        .map(|event| keys.feed(*event, Grammar::Objects))
        .map(|input| input.map(|i| (i.action, i.count, i.register)))
        .collect();
      assert_eq!(got, want, "{events:?}");
    }
  }

  #[test]
  fn yank_still_takes_p() {
    let yank = |register| Some((Action::Yank, register));
    let path = |register| Some((Action::YankPath, register));
    let cases = [
      (Grammar::Normal, "yp", vec![yank('"'), path('"')]),
      (Grammar::Cursor, "yp", vec![yank('"'), path('"')]),
      (Grammar::Normal, "\"ayp", vec![None, None, yank('a'), path('a')]),
      (Grammar::Normal, "yy", vec![yank('"'), yank('"')]),
      (Grammar::Cursor, "yY", vec![yank('"'), Some((Action::YankRaw, '"'))]),
      (Grammar::Objects, "yp", vec![yank('"'), None]),
      (Grammar::Other, "yp", vec![yank('"'), None]),
    ];
    for (grammar, typed, want) in cases {
      let mut keys = Keys::default();
      let got: Vec<_> = typed
        .chars()
        .map(|ch| keys.feed(key(ch), grammar))
        .map(|input| input.map(|i| (i.action, i.register)))
        .collect();
      assert_eq!(got, want, "{typed}");
    }
  }

  #[test]
  fn objects_accept_quotes_counts_and_aliases() {
    for (name, kind) in [
      ("w", O::Word),
      ("W", O::BigWord),
      ("s", O::Sentence),
      ("p", O::Paragraph),
      ("(", O::Parentheses),
      (")", O::Parentheses),
      ("b", O::Parentheses),
      ("[", O::Brackets),
      ("]", O::Brackets),
      ("{", O::Braces),
      ("}", O::Braces),
      ("B", O::Braces),
      ("<", O::Angles),
      (">", O::Angles),
      ("\"", O::DoubleQuote),
      ("'", O::SingleQuote),
      ("“", O::DoubleQuote),
      ("’", O::SingleQuote),
    ] {
      for (prefix, around) in [("i", false), ("a", true)] {
        let mut keys = Keys::default();
        let input = format!("\"b3{prefix}{name}")
          .chars()
          .filter_map(|ch| keys.feed(key(ch), Grammar::Objects))
          .last()
          .unwrap();
        assert_eq!(input.action, Action::Object(kind, around));
        assert_eq!(input.count, 3);
        assert_eq!(input.register, 'b');
        assert!(clues(prefix).iter().any(|(key, _)| key == name));
      }
    }
  }

  #[test]
  fn cursor_and_object_grammar() {
    let grammars =
      [Grammar::Normal, Grammar::Cursor, Grammar::Objects, Grammar::Other];
    for grammar in grammars {
      let mut keys = Keys::default();
      let input = keys.feed(key('i'), grammar);
      match grammar {
        Grammar::Normal => {
          assert_eq!(input.unwrap().action, Action::Cursor);
          assert!(keys.awaiting().is_none());
        }
        Grammar::Objects => {
          assert!(input.is_none());
          assert_eq!(keys.awaiting(), Some("i"));
          assert_eq!(
            keys.feed(key('w'), grammar).unwrap().action,
            Action::Object(O::Word, false)
          );
        }
        Grammar::Cursor | Grammar::Other => {
          assert!(input.is_none());
          assert!(keys.awaiting().is_none());
        }
      }
      keys.clear();
      keys.feed(key('a'), grammar);
      assert_eq!(keys.awaiting(), (grammar == Grammar::Objects).then_some("a"));
      keys.clear();
      for ch in "\"i2mi".chars() {
        let input = keys.feed(key(ch), grammar);
        if let Some(input) = input {
          assert_eq!(input.action, Action::Mark('i'));
          assert_eq!(input.register, 'i');
          assert_eq!(input.count, 2);
        }
      }
      assert_eq!(keys.pending(), "");
      keys.feed(key('g'), grammar);
      assert!(keys.feed(key('i'), grammar).is_none());
      assert_eq!(keys.pending(), "");
    }
  }

  #[test]
  fn escape_resets_pending_count() {
    let mut keys = Keys::default();
    keys.feed(key('9'), Grammar::Objects);
    keys
      .feed(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), Grammar::Objects);
    assert_eq!(keys.feed(key('j'), Grammar::Objects).unwrap().count, 1);
    assert!(keys.feed(key('d'), Grammar::Objects).is_none());
    assert_eq!(
      keys.feed(key(' '), Grammar::Objects).unwrap().action,
      Action::Motion(Motion::NextPage)
    );
    let shifted = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::SHIFT);
    assert_eq!(
      keys.feed(shifted, Grammar::Objects).unwrap().action,
      Action::Motion(Motion::PreviousPage)
    );
  }
}
