use crate::split::{Axis, Direction as D};
use crate::window::Window as W;
use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};
use vpdf_reader::motion::Motion;
use vpdf_reader::motion::Motion as M;
use vpdf_reader::objects::Object as O;
use vpdf_reader::reader::SelectionKind;

#[derive(
  Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "lowercase")]
pub enum PanelKind {
  Outline,
  Figures,
  Results,
  Text,
  Version,
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
      Self::Figures => "FIGURES",
      Self::Results => "RESULTS",
      Self::Text => "TEXT",
      Self::Version => "VERSION",
      Self::Info => "INFO",
      Self::Recent => "RECENT",
      Self::Marks => "MARKS",
      Self::Jumps => "JUMPS",
      Self::Annotations => "ANNOTATIONS",
    }
  }

  /// Rows group under headings the panel owns.
  pub fn grouped(self) -> bool {
    matches!(
      self,
      Self::Results
        | Self::Annotations
        | Self::Marks
        | Self::Info
        | Self::Version
    )
  }

  /// Groups that start closed.
  pub fn folded(self) -> &'static [&'static str] {
    match self {
      Self::Info => &[crate::panel::RENDER, crate::panel::TERMINAL],
      Self::Version => &[crate::panel::BUILD],
      _ => &[],
    }
  }

  /// List rows keep a gutter for the current-row marker.
  pub fn marks_current(self) -> bool {
    matches!(self, Self::Figures | Self::Jumps)
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fold {
  Close,
  Open,
  ExpandAll,
  CollapseAll,
}

impl Fold {
  pub fn of(action: Action) -> Option<Self> {
    match action {
      Action::Fold(fold) => Some(fold),
      Action::Motion(Motion::Left) => Some(Self::Close),
      Action::Motion(Motion::Right) => Some(Self::Open),
      _ => None,
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
  Help,
  Motion(Motion),
  Section(bool),
  Figure(bool),
  FigureFit,
  Region,
  RegionStart,
  RegionShift(i8, i8),
  Yank,
  YankLine,
  /// `yp`: the document's full path.
  YankPath,
  /// `yY`: lines as laid out, without joining.
  YankRaw,
  Object(vpdf_reader::objects::Object, bool),
  Visual(SelectionKind),
  Cursor,
  Swap,
  Reselect,
  Cancel,
  Quit,
  Alternate,
  Search(bool),
  WordSearch,
  Definition,
  Result(bool),
  Panel(PanelKind),
  ClosePanel,
  Fold(Fold),
  OutlineAuto,
  RecentToggle,
  RecentRemove,
  LocalFocus,
  Follow,
  OpenUrl,
  History(bool),
  Command,
  Tab(bool),
  /// Link hint action.
  Hints(crate::hints::Mode),
  Window(crate::window::Window),
  /// Preview the link under the cursor.
  Peek,
  /// Page thumbnail grid.
  Overview,
  /// Context menu at the focused item.
  Menu,
  Mark(char),
  /// Mark name and whether to keep the glyph column.
  GoMark(char, bool),
  Scroll(f32),
  Screen(i32),
  Rows(i32),
  Align(f32),
  Zoom(i32),
  Fit,
  FitPage,
  Rotate,
  Crop,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
  pub action: Action,
  pub count: usize,
  pub explicit: bool,
  pub register: char,
}

impl Input {
  /// `action` with no count or register.
  pub const fn plain(action: Action) -> Self {
    Self { action, count: 1, explicit: false, register: '"' }
  }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Grammar {
  Normal,
  /// Cursor mode in the content, without an operator.
  Cursor,
  Objects,
  Panel,
  Other,
}

mod dispatch;
pub use dispatch::{Dispatch, Keys};

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

pub fn cancels(key: KeyEvent) -> bool {
  key.code == KeyCode::Esc
    || (key.code == KeyCode::Char('c')
      && key.modifiers == KeyModifiers::CONTROL)
}

/// Keys a binding name types; `token`'s inverse.
pub fn parse(name: &str) -> Option<Vec<KeyEvent>> {
  let key = KeyEvent::new;
  let none = KeyModifiers::NONE;
  let named = [
    ("S-Space", key(KeyCode::Char(' '), KeyModifiers::SHIFT)),
    ("Space", key(KeyCode::Char(' '), none)),
    ("Enter", key(KeyCode::Enter, none)),
    ("Tab", key(KeyCode::Tab, none)),
    ("BS", key(KeyCode::Backspace, none)),
  ];
  let mut keys = Vec::new();
  let mut rest = name;
  while let Some(first) = rest.chars().next() {
    if let Some((name, event)) =
      named.iter().find(|(name, _)| rest.starts_with(name))
    {
      keys.push(*event);
      rest = &rest[name.len()..];
    } else if let Some(tail) = rest.strip_prefix("C-")
      && let Some(ch) = tail.chars().next()
    {
      keys.push(key(KeyCode::Char(ch), KeyModifiers::CONTROL));
      rest = &tail[ch.len_utf8()..];
    } else {
      keys.push(key(KeyCode::Char(first), none));
      rest = &rest[first.len_utf8()..];
    }
  }
  (!keys.is_empty()).then_some(keys)
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

/// Keys, action, and description, also shown as prefix-key clues.
pub(crate) const BINDINGS: &[(&str, Action, &str)] = &[
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
  (" ", Action::Screen(1), "screen down with overlap"),
  ("S-Space", Action::Screen(-1), "screen up with overlap"),
  ("C-n", Action::Motion(M::NextPage), "next page"),
  ("C-p", Action::Motion(M::PreviousPage), "previous page"),
  ("BS", Action::Motion(M::PreviousPage), "previous page"),
  ("y", Action::Yank, "yank"),
  ("yy", Action::YankLine, "yank line"),
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
  ("ZZ", Action::Window(crate::window::Window::Close), "close pane"),
  ("ZQ", Action::Window(crate::window::Window::Close), "close pane"),
  ("C-^", Action::Alternate, "alternate document"),
  ("C-6", Action::Alternate, "alternate document"),
  ("C-c", Action::Cancel, "cancel"),
  ("/", Action::Search(false), "search forward"),
  ("?", Action::Search(true), "search backward"),
  ("*", Action::WordSearch, "search cursor word"),
  ("n", Action::Result(false), "next match"),
  ("N", Action::Result(true), "previous match"),
  ("go", Action::Panel(PanelKind::Outline), "table of contents"),
  ("gs", Action::Panel(PanelKind::Results), "search results"),
  ("gf", Action::Panel(PanelKind::Figures), "figures"),
  ("gr", Action::Panel(PanelKind::Recent), "recent documents"),
  ("ga", Action::Panel(PanelKind::Annotations), "annotations"),
  ("gd", Action::Definition, "go to definition"),
  ("gx", Action::OpenUrl, "open URL"),
  ("g?", Action::Help, "key reference"),
  ("gq", Action::ClosePanel, "close panel"),
  ("K", Action::Peek, "preview link or annotation note"),
  ("gp", Action::Overview, "page thumbnails"),
  ("]f", Action::Figure(true), "next figure"),
  ("[f", Action::Figure(false), "previous figure"),
  ("zF", Action::FigureFit, "fit figure"),
  ("gF", Action::Region, "figure actions"),
  ("R", Action::RegionStart, "capture rectangle"),
  ("f", Action::Hints(crate::hints::Mode::Follow), "link hints"),
  (
    "F",
    Action::Hints(crate::hints::Mode::Cursor),
    "link hints, enter Cursor mode",
  ),
  (
    "yf",
    Action::Hints(crate::hints::Mode::Copy),
    "link hints, copy reference URI",
  ),
  ("gt", Action::Tab(true), "next tab, or [count] tab"),
  ("gT", Action::Tab(false), "previous tab"),
  ("Enter", Action::Follow, "follow link"),
  ("C-o", Action::History(false), "jump back"),
  ("C-i", Action::History(true), "jump forward"),
  ("Tab", Action::LocalFocus, "toggle content/panel focus"),
  (":", Action::Command, "command line"),
  ("C-d", Action::Scroll(0.5), "half page down"),
  ("d", Action::Scroll(0.5), "half page down"),
  ("C-u", Action::Scroll(-0.5), "half page up"),
  ("u", Action::Scroll(-0.5), "half page up"),
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
  ("zR", Action::Fold(Fold::ExpandAll), "expand all branches"),
  ("zM", Action::Fold(Fold::CollapseAll), "collapse all branches"),
  ("za", Action::OutlineAuto, "toggle automatic expansion"),
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
  ("C-rh", Action::Window(W::Extend(D::Left)), "resize pane left"),
  ("C-rl", Action::Window(W::Extend(D::Right)), "resize pane right"),
  ("C-rk", Action::Window(W::Extend(D::Up)), "resize pane up"),
  ("C-rj", Action::Window(W::Extend(D::Down)), "resize pane down"),
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

pub const RECENT_BINDINGS: &[(&str, Action, &str)] = &[
  (" ", Action::RecentToggle, "toggle recent checkbox"),
  ("x", Action::RecentRemove, "remove recent files or a mark"),
];

/// The first key bound to `action`.
pub fn key_for(action: Action) -> Option<&'static str> {
  BINDINGS
    .iter()
    .chain(RECENT_BINDINGS)
    .find(|(_, bound, _)| *bound == action)
    .map(|(keys, ..)| *keys)
}

pub fn binding(key: &str) -> Option<Action> {
  BINDINGS.iter().find(|(keys, ..)| *keys == key).map(|(_, action, _)| *action)
}

/// A binding name in help notation: `C-o` as `<C-o>`, `Enter` as `<CR>`;
/// names it cannot write stay as they are.
pub fn notation(name: &str) -> String {
  let keys = parse(name).unwrap_or_default();
  let shown = keys.into_iter().map(|key| match key.code {
    KeyCode::Char(ch) if key.modifiers == KeyModifiers::CONTROL => {
      Some(format!("<C-{ch}>"))
    }
    KeyCode::Char(' ') if key.modifiers == KeyModifiers::SHIFT => {
      Some("<S-Space>".into())
    }
    KeyCode::Char(' ') => Some("<Space>".into()),
    KeyCode::Char('<') => Some("<lt>".into()),
    KeyCode::Char(ch) => Some(ch.to_string()),
    KeyCode::Enter => Some("<CR>".into()),
    KeyCode::Tab => Some("<Tab>".into()),
    KeyCode::Backspace => Some("<BS>".into()),
    _ => None,
  });
  shown.collect::<Option<String>>().unwrap_or_else(|| name.to_owned())
}

#[cfg(test)]
mod tests;
