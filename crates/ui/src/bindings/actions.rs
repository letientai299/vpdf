use crate::keys::{Action, Fold, PanelKind};
use crate::window::Window;
use vpdf_reader::motion::Motion;
use vpdf_reader::objects::Object;
use vpdf_reader::reader::SelectionKind;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Argument {
  None,
  Motion(Motion),
  Object { kind: Object, around: bool },
  Number(f32),
  Integer(i32),
  Character(char),
  Mark { name: char, exact: bool },
  Shift { dx: i8, dy: i8 },
  Window(Window),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
  Fixed(Action),
  Motion,
  Object,
  Scroll,
  Screen,
  Rows,
  Align,
  Zoom,
  Mark,
  GoMark,
  RegionShift,
  Window,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spec {
  pub name: &'static str,
  pub kind: Kind,
}

impl Spec {
  pub fn resolve(self, argument: Argument) -> crate::Result<Action> {
    Ok(match (self.kind, argument) {
      (Kind::Fixed(action), Argument::None) => action,
      (Kind::Motion, Argument::Motion(value)) => Action::Motion(value),
      (Kind::Object, Argument::Object { kind, around }) => {
        Action::Object(kind, around)
      }
      (Kind::Scroll, Argument::Number(value)) if value.is_finite() => {
        Action::Scroll(value)
      }
      (Kind::Screen, Argument::Integer(value)) => Action::Screen(value),
      (Kind::Rows, Argument::Integer(value)) => Action::Rows(value),
      (Kind::Align, Argument::Number(value))
        if value.is_finite() && (0.0..=1.0).contains(&value) =>
      {
        Action::Align(value)
      }
      (Kind::Zoom, Argument::Integer(value)) => Action::Zoom(value),
      (Kind::Mark, Argument::Character(value))
        if value.is_ascii_alphabetic() =>
      {
        Action::Mark(value)
      }
      (Kind::GoMark, Argument::Mark { name, exact })
        if name.is_ascii_alphabetic() =>
      {
        Action::GoMark(name, exact)
      }
      (Kind::RegionShift, Argument::Shift { dx, dy }) => {
        Action::RegionShift(dx, dy)
      }
      (Kind::Window, Argument::Window(value)) => Action::Window(value),
      _ => anyhow::bail!("Invalid argument for {}", self.name),
    })
  }
}

pub fn lookup(name: &str) -> Option<Spec> {
  ACTIONS.iter().find(|spec| spec.name == name).copied()
}

pub fn describe(action: Action) -> Option<(Spec, Argument)> {
  let (kind, argument) = match action {
    Action::Motion(value) => (Kind::Motion, Argument::Motion(value)),
    Action::Object(kind, around) => {
      (Kind::Object, Argument::Object { kind, around })
    }
    Action::Scroll(value) => (Kind::Scroll, Argument::Number(value)),
    Action::Screen(value) => (Kind::Screen, Argument::Integer(value)),
    Action::Rows(value) => (Kind::Rows, Argument::Integer(value)),
    Action::Align(value) => (Kind::Align, Argument::Number(value)),
    Action::Zoom(value) => (Kind::Zoom, Argument::Integer(value)),
    Action::Mark(value) => (Kind::Mark, Argument::Character(value)),
    Action::GoMark(name, exact) => {
      (Kind::GoMark, Argument::Mark { name, exact })
    }
    Action::RegionShift(dx, dy) => {
      (Kind::RegionShift, Argument::Shift { dx, dy })
    }
    Action::Window(value) => (Kind::Window, Argument::Window(value)),
    _ => (Kind::Fixed(action), Argument::None),
  };
  let spec = ACTIONS.iter().find(|spec| spec.kind == kind).copied()?;
  Some((spec, argument))
}

macro_rules! specs {
  ($($name:literal => $kind:expr),* $(,)?) => {
    pub const ACTIONS: &[Spec] = &[$(Spec { name: $name, kind: $kind }),*];

pub const MOTIONS: &[(&str, Motion)] = &[
  ("left", Motion::Left),
  ("right", Motion::Right),
  ("down", Motion::Down),
  ("up", Motion::Up),
  ("word", Motion::Word),
  ("back-word", Motion::BackWord),
  ("end-word", Motion::EndWord),
  ("big-word", Motion::BigWord),
  ("back-big-word", Motion::BackBigWord),
  ("end-big-word", Motion::EndBigWord),
  ("previous-big-end", Motion::PreviousBigEnd),
  ("next-sentence", Motion::NextSentence),
  ("previous-sentence", Motion::PreviousSentence),
  ("next-paragraph", Motion::NextParagraph),
  ("previous-paragraph", Motion::PreviousParagraph),
  ("line-start", Motion::LineStart),
  ("first-nonblank", Motion::FirstNonblank),
  ("line-end", Motion::LineEnd),
  ("first", Motion::First),
  ("last", Motion::Last),
  ("next-page", Motion::NextPage),
  ("previous-page", Motion::PreviousPage),
];

pub const OBJECTS: &[(&str, Object)] = &[
  ("word", Object::Word),
  ("big-word", Object::BigWord),
  ("sentence", Object::Sentence),
  ("paragraph", Object::Paragraph),
  ("parentheses", Object::Parentheses),
  ("brackets", Object::Brackets),
  ("braces", Object::Braces),
  ("angles", Object::Angles),
  ("double-quote", Object::DoubleQuote),
  ("single-quote", Object::SingleQuote),
];
  };
}

specs! {
  "recent.toggle" => Kind::Fixed(Action::RecentToggle),
  "recent.remove" => Kind::Fixed(Action::RecentRemove),
  "motion" => Kind::Motion,
  "object" => Kind::Object,
  "scroll" => Kind::Scroll,
  "screen" => Kind::Screen,
  "rows" => Kind::Rows,
  "align" => Kind::Align,
  "zoom" => Kind::Zoom,
  "mark.set" => Kind::Mark,
  "mark.go" => Kind::GoMark,
  "region.shift" => Kind::RegionShift,
  "window" => Kind::Window,
  "section.previous" => Kind::Fixed(Action::Section(false)),
  "section.next" => Kind::Fixed(Action::Section(true)),
  "figure.previous" => Kind::Fixed(Action::Figure(false)),
  "figure.next" => Kind::Fixed(Action::Figure(true)),
  "figure.fit" => Kind::Fixed(Action::FigureFit),
  "region.open" => Kind::Fixed(Action::Region),
  "region.start" => Kind::Fixed(Action::RegionStart),
  "yank" => Kind::Fixed(Action::Yank),
  "yank.line" => Kind::Fixed(Action::YankLine),
  "yank.path" => Kind::Fixed(Action::YankPath),
  "yank.raw" => Kind::Fixed(Action::YankRaw),
  "visual.character" => Kind::Fixed(Action::Visual(SelectionKind::Character)),
  "visual.line" => Kind::Fixed(Action::Visual(SelectionKind::Line)),
  "cursor" => Kind::Fixed(Action::Cursor),
  "selection.swap" => Kind::Fixed(Action::Swap),
  "selection.restore" => Kind::Fixed(Action::Reselect),
  "cancel" => Kind::Fixed(Action::Cancel),
  "quit" => Kind::Fixed(Action::Quit),
  "search.forward" => Kind::Fixed(Action::Search(false)),
  "search.backward" => Kind::Fixed(Action::Search(true)),
  "search.word" => Kind::Fixed(Action::WordSearch),
  "definition" => Kind::Fixed(Action::Definition),
  "result.next" => Kind::Fixed(Action::Result(false)),
  "result.previous" => Kind::Fixed(Action::Result(true)),
  "panel.outline" => Kind::Fixed(Action::Panel(PanelKind::Outline)),
  "panel.figures" => Kind::Fixed(Action::Panel(PanelKind::Figures)),
  "panel.results" => Kind::Fixed(Action::Panel(PanelKind::Results)),
  "panel.links" => Kind::Fixed(Action::Panel(PanelKind::Links)),
  "panel.text" => Kind::Fixed(Action::Panel(PanelKind::Text)),
  "panel.help" => Kind::Fixed(Action::Help),
  "panel.version" => Kind::Fixed(Action::Panel(PanelKind::Version)),
  "panel.info" => Kind::Fixed(Action::Panel(PanelKind::Info)),
  "panel.recent" => Kind::Fixed(Action::Panel(PanelKind::Recent)),
  "panel.marks" => Kind::Fixed(Action::Panel(PanelKind::Marks)),
  "panel.jumps" => Kind::Fixed(Action::Panel(PanelKind::Jumps)),
  "panel.annotations" => Kind::Fixed(Action::Panel(PanelKind::Annotations)),
  "outline.close" => Kind::Fixed(Action::Fold(Fold::Close)),
  "outline.open" => Kind::Fixed(Action::Fold(Fold::Open)),
  "outline.expand-all" => Kind::Fixed(Action::Fold(Fold::ExpandAll)),
  "outline.collapse-all" => Kind::Fixed(Action::Fold(Fold::CollapseAll)),
  "panel.close" => Kind::Fixed(Action::ClosePanel),
  "focus.toggle" => Kind::Fixed(Action::LocalFocus),
  "follow" => Kind::Fixed(Action::Follow),
  "url.open" => Kind::Fixed(Action::OpenUrl),
  "history.next" => Kind::Fixed(Action::History(true)),
  "history.previous" => Kind::Fixed(Action::History(false)),
  "command" => Kind::Fixed(Action::Command),
  "tab.next" => Kind::Fixed(Action::Tab(true)),
  "tab.previous" => Kind::Fixed(Action::Tab(false)),
  "hints.open" => Kind::Fixed(Action::Hints(false)),
  "hints.copy" => Kind::Fixed(Action::Hints(true)),
  "preview" => Kind::Fixed(Action::Peek),
  "overview" => Kind::Fixed(Action::Overview),
  "fit.width" => Kind::Fixed(Action::Fit),
  "fit.page" => Kind::Fixed(Action::FitPage),
  "rotate" => Kind::Fixed(Action::Rotate),
  "crop" => Kind::Fixed(Action::Crop),
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn arguments_validate_before_dispatch() {
    for (name, argument) in [
      ("tab.next", Argument::Integer(1)),
      ("motion", Argument::None),
      ("scroll", Argument::Number(f32::NAN)),
      ("align", Argument::Number(1.1)),
      ("mark.set", Argument::Character('1')),
    ] {
      assert!(lookup(name).unwrap().resolve(argument).is_err(), "{name}");
    }
    assert_eq!(
      lookup("tab.next").unwrap().resolve(Argument::None).unwrap(),
      Action::Tab(true)
    );
    assert!(lookup("missing").is_none());
  }
}
