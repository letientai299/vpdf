use super::InputOwner;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub enum Cancel {
  Disarm,
  Overlay,
  Prompt,
  Reset(Reset),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reset {
  Prompt,
  Hints,
  Overview,
  Pending,
  Content,
  Selection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub enum Dismiss {
  None,
  Prompt,
  Hints,
  Peek,
  Overlay,
  Reading,
}

impl InputOwner {
  /// Presence includes previews without keyboard focus.
  pub fn cancel(self, pending: bool, present: impl Fn(Self) -> bool) -> Cancel {
    if matches!(self, Self::Peek | Self::Region | Self::Context) {
      return if pending { Cancel::Disarm } else { Cancel::Overlay };
    }
    if self == Self::Prompt && (present(Self::Peek) || present(Self::Region)) {
      return Cancel::Prompt;
    }
    Cancel::Reset(match self {
      Self::Prompt => Reset::Prompt,
      Self::Hints => Reset::Hints,
      _ if present(Self::Overview) => Reset::Overview,
      Self::Content if pending => Reset::Pending,
      Self::Content => Reset::Content,
      _ => Reset::Selection,
    })
  }

  pub fn dismiss(self, clues: bool) -> Dismiss {
    match self {
      Self::Prompt => Dismiss::Prompt,
      Self::Hints => Dismiss::Hints,
      Self::Peek => Dismiss::Peek,
      Self::Region | Self::Context => Dismiss::Overlay,
      _ if clues => Dismiss::Reading,
      _ => Dismiss::None,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn pending_preserves_overlays() {
    for owner in [InputOwner::Peek, InputOwner::Region, InputOwner::Context] {
      assert_eq!(owner.cancel(true, |_| true), Cancel::Disarm);
      assert_eq!(owner.cancel(false, |_| true), Cancel::Overlay);
    }
  }

  #[test]
  fn prompt_preserves_underlays() {
    for underlying in [InputOwner::Peek, InputOwner::Region] {
      for pending in [false, true] {
        assert_eq!(
          InputOwner::Prompt.cancel(pending, |owner| owner == underlying),
          Cancel::Prompt
        );
      }
    }
    assert_eq!(
      InputOwner::Prompt.cancel(false, |owner| owner == InputOwner::Overview),
      Cancel::Reset(Reset::Prompt)
    );
    assert_eq!(
      InputOwner::Hints.cancel(true, |_| true),
      Cancel::Reset(Reset::Hints)
    );
  }

  #[test]
  fn reset_respects_focus() {
    for owner in [InputOwner::Content, InputOwner::Panel, InputOwner::Overview]
    {
      for pending in [false, true] {
        assert_eq!(
          owner.cancel(pending, |owner| owner == InputOwner::Overview),
          Cancel::Reset(Reset::Overview)
        );
      }
    }
    assert_eq!(
      InputOwner::Content.cancel(true, |_| false),
      Cancel::Reset(Reset::Pending)
    );
    assert_eq!(
      InputOwner::Content.cancel(false, |_| false),
      Cancel::Reset(Reset::Content)
    );
    assert_eq!(
      InputOwner::Panel.cancel(true, |_| false),
      Cancel::Reset(Reset::Selection)
    );
  }

  #[test]
  fn click_dismissal_respects_focus() {
    for (owner, expected) in [
      (InputOwner::Prompt, Dismiss::Prompt),
      (InputOwner::Hints, Dismiss::Hints),
      (InputOwner::Peek, Dismiss::Peek),
      (InputOwner::Region, Dismiss::Overlay),
      (InputOwner::Context, Dismiss::Overlay),
    ] {
      for clues in [false, true] {
        assert_eq!(owner.dismiss(clues), expected);
      }
    }
    for owner in [InputOwner::Panel, InputOwner::Overview, InputOwner::Content]
    {
      assert_eq!(owner.dismiss(false), Dismiss::None);
      assert_eq!(owner.dismiss(true), Dismiss::Reading);
    }
  }
}
