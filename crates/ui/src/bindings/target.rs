use super::{Context, PromptAction};
use crate::keys::Action;
use serde::{Deserialize, Serialize};
use vpdf_reader::objects::Object;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "action", deny_unknown_fields)]
pub enum Control {
  #[serde(rename = "count.digit")]
  Digit { digit: u8 },
  #[serde(rename = "register.choose")]
  Register,
  #[serde(rename = "mark.choose")]
  SetMark,
  #[serde(rename = "mark.jump")]
  GoMark { exact: bool },
  #[serde(rename = "object.begin")]
  Object { around: bool },
  #[serde(rename = "window.resize")]
  Resize,
  #[serde(rename = "yank.begin")]
  Yank,
}

impl Control {
  pub const NAMES: &[&str] = &[
    "count.digit",
    "register.choose",
    "mark.choose",
    "mark.jump",
    "object.begin",
    "window.resize",
    "yank.begin",
  ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "action", deny_unknown_fields)]
pub enum HintAction {
  #[serde(rename = "hint.type")]
  Type { character: char },
  #[serde(rename = "hint.erase")]
  Erase,
  #[serde(rename = "hint.follow")]
  Follow,
  #[serde(rename = "hint.close")]
  Close,
  #[serde(rename = "hint.cancel")]
  Cancel,
}

impl HintAction {
  pub const NAMES: &[&str] =
    &["hint.type", "hint.erase", "hint.follow", "hint.close", "hint.cancel"];
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
  Reader(Action),
  Prompt(PromptAction),
  Hint(HintAction),
  Control(Control),
  TextObject(Object),
  Command(String),
}

impl Target {
  pub fn cancels(&self) -> bool {
    matches!(
      self,
      Self::Reader(Action::Cancel)
        | Self::Prompt(PromptAction::Cancel)
        | Self::Hint(HintAction::Cancel)
    )
  }

  pub fn allowed(&self, context: Context) -> bool {
    use Context as C;
    let context = match context {
      C::Recent => C::Panel,
      C::HomeRecent => C::Home,
      other => other,
    };
    match self {
      Self::Prompt(PromptAction::Editor) => {
        matches!(context, C::Prompt | C::Command | C::Search)
      }
      Self::Prompt(_) => {
        matches!(context, C::Prompt | C::Command | C::Search | C::Password)
      }
      Self::Hint(HintAction::Type { character }) => {
        context == C::Hints && character.is_ascii_alphabetic()
      }
      Self::Hint(_) => context == C::Hints,
      Self::TextObject(_) => context == C::Object,
      Self::Command(_) => !matches!(
        context,
        C::Prompt
          | C::Password
          | C::Object
          | C::Resize
          | C::Register
          | C::MarkSet
          | C::MarkJump
      ),
      Self::Control(Control::Digit { digit }) => {
        *digit <= 9
          && !matches!(
            context,
            C::Prompt
              | C::Command
              | C::Search
              | C::Password
              | C::Hints
              | C::Register
              | C::MarkSet
              | C::MarkJump
          )
      }
      Self::Control(Control::Object { .. }) => {
        matches!(context, C::Cursor | C::Visual | C::Operator)
      }
      Self::Control(Control::Register | Control::Yank) => matches!(
        context,
        C::Normal | C::Cursor | C::Visual | C::Operator | C::Panel
      ),
      Self::Control(Control::SetMark | Control::GoMark { .. }) => {
        matches!(
          context,
          C::Normal | C::Cursor | C::Visual | C::Operator | C::Panel
        )
      }
      Self::Control(Control::Resize) => matches!(
        context,
        C::Home
          | C::Normal
          | C::Cursor
          | C::Visual
          | C::Operator
          | C::Panel
          | C::Overview
          | C::Preview
          | C::Menu
          | C::Region
      ),
      Self::Reader(action) => reader_allowed(*action, context),
    }
  }
}

fn reader_allowed(action: Action, context: Context) -> bool {
  use Context as C;
  match action {
    Action::Fold(_) | Action::OutlineAuto => context == C::Panel,
    Action::Motion(
      vpdf_reader::motion::Motion::Left | vpdf_reader::motion::Motion::Right,
    ) if context == C::Panel => true,
    Action::RecentToggle | Action::RecentRemove => {
      matches!(context, C::Recent | C::HomeRecent | C::Panel | C::Home)
    }

    Action::History(_) if context == C::MarkJump => true,
    Action::Help | Action::Cancel => !matches!(
      context,
      C::Prompt | C::Command | C::Search | C::Password | C::Hints
    ),
    Action::Menu => matches!(
      context,
      C::Normal
        | C::Cursor
        | C::Visual
        | C::Panel
        | C::Recent
        | C::Overview
        | C::Menu
    ),
    Action::Window(crate::window::Window::Extend(_))
      if context == C::Resize =>
    {
      true
    }
    action if context == C::Menu => crate::context::accepts(action),
    action if context == C::Region => crate::region::accepts(action),
    action if context == C::Preview => crate::preview::Preview::allows(action),
    action if context == C::Overview => {
      crate::overview::Overview::allows(action)
    }
    action if context == C::Panel => {
      crate::panel::Panel::new(crate::keys::PanelKind::Annotations, Vec::new())
        .allows(action)
    }
    action if context == C::Home => {
      matches!(
        action,
        Action::Follow
          | Action::Swap
          | Action::Command
          | Action::Search(_)
          | Action::Quit
          | Action::Alternate
          | Action::Window(_)
          | Action::Tab(_)
      ) || crate::panel::Panel::new(crate::keys::PanelKind::Recent, Vec::new())
        .navigate_available(action)
    }
    _ => !matches!(
      context,
      C::Prompt
        | C::Command
        | C::Search
        | C::Password
        | C::Object
        | C::Hints
        | C::Resize
        | C::Register
        | C::MarkSet
        | C::MarkJump
    ),
  }
}
