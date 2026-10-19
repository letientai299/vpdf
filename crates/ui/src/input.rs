mod cancel;
#[cfg(test)]
mod contracts;
pub use cancel::{Cancel, Dismiss, Reset};
pub mod edge;
pub mod gesture;
pub mod grab;
pub mod links;
pub mod mouse;
mod overlay;
pub use overlay::Overlay;
#[cfg(test)]
mod overlay_tests;
mod targets;
pub use targets::Targets;
pub mod wheel;
use crate::keys::{Action, Grammar};
use vpdf_reader::reader::Mode;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputOwner {
  Prompt,
  Hints,
  Context,
  Peek,
  Region,
  Panel,
  Overview,
  Content,
}

impl InputOwner {
  pub fn resolve(active: impl Fn(Self) -> bool) -> Self {
    [
      Self::Prompt,
      Self::Hints,
      Self::Region,
      Self::Context,
      Self::Peek,
      Self::Panel,
      Self::Overview,
    ]
    .into_iter()
    .find(|&owner| active(owner))
    .unwrap_or(Self::Content)
  }

  pub fn is_overlay(self) -> bool {
    !matches!(self, Self::Content | Self::Panel | Self::Overview)
  }

  pub fn grammar(self, mode: Mode, pending_operator: bool) -> Grammar {
    match self {
      Self::Content if matches!(mode, Mode::Visual(_)) || pending_operator => {
        Grammar::Objects
      }
      Self::Content if mode == Mode::Normal => Grammar::Normal,
      Self::Content => Grammar::Cursor,
      Self::Panel => Grammar::Panel,
      _ => Grammar::Other,
    }
  }
}

pub(crate) fn transfers_overlay(action: Action) -> bool {
  matches!(
    action,
    Action::Window(_)
      | Action::Tab(_)
      | Action::LocalFocus
      | Action::History(_)
      | Action::Panel(_)
      | Action::ClosePanel
  )
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_reader::reader::SelectionKind;

  #[test]
  fn overlays_preserve_underlying_focus() {
    let mut active =
      vec![InputOwner::Panel, InputOwner::Overview, InputOwner::Context];
    let owner = |active: &[InputOwner]| {
      InputOwner::resolve(|owner| active.contains(&owner))
    };
    assert_eq!(owner(&active), InputOwner::Context);
    active.push(InputOwner::Region);
    assert_eq!(owner(&active), InputOwner::Region);
    active.push(InputOwner::Prompt);
    assert_eq!(owner(&active), InputOwner::Prompt);
    active.retain(|owner| !owner.is_overlay());
    assert_eq!(owner(&active), InputOwner::Panel);
    active.retain(|owner| *owner != InputOwner::Panel);
    assert_eq!(owner(&active), InputOwner::Overview);
    assert_eq!(owner(&[]), InputOwner::Content);
  }

  #[test]
  fn content_modes_do_not_change_panel_grammar() {
    for (mode, pending, expected) in [
      (Mode::Normal, false, Grammar::Normal),
      (Mode::Cursor, false, Grammar::Cursor),
      (Mode::Visual(SelectionKind::Character), false, Grammar::Objects),
      (Mode::Normal, true, Grammar::Objects),
    ] {
      assert!(InputOwner::Content.grammar(mode, pending) == expected);
      assert!(InputOwner::Panel.grammar(mode, pending) == Grammar::Panel);
      assert!(InputOwner::Peek.grammar(mode, pending) == Grammar::Other);
    }
  }
}
