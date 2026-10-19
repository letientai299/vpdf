use super::{Action, App, BAR_HINT, InputOwner, Origin, Result};

impl App {
  pub fn input_owner(&self) -> InputOwner {
    InputOwner::resolve(|owner| match owner {
      InputOwner::Prompt => self.prompt.is_some(),
      InputOwner::Hints => self.hints.is_some(),
      InputOwner::Region => {
        self.region.as_ref().is_some_and(vpdf_ui::region::Region::is_bar)
      }
      InputOwner::Context => self.context_menu.is_some(),
      InputOwner::Peek => {
        self.peek.as_ref().is_some_and(|peek| peek.origin() == Origin::Key)
      }
      InputOwner::Panel => self.panel_focused(),
      InputOwner::Overview => self.overview.is_some(),
      InputOwner::Content => true,
    })
  }

  pub(crate) fn dismiss_overlay(&mut self) {
    self.context_menu = None;
    self.end_gestures();
    self.peek = None;
    // The bar's key hint goes with it.
    if self.region.take().is_some() && self.reader.message == BAR_HINT {
      self.reader.notice("");
    }
  }

  pub(crate) fn overlay_transfer(&mut self, action: Action) -> bool {
    if !matches!(
      action,
      Action::Window(_)
        | Action::Tab(_)
        | Action::LocalFocus
        | Action::History(_)
        | Action::Panel(_)
        | Action::ClosePanel
    ) {
      return false;
    }
    self.dismiss_overlay();
    true
  }

  /// Ends pending keys and notes.
  pub(crate) fn interrupt(&mut self) {
    self.disarm();
    self.clear_notes();
  }

  /// Drops the error and the message.
  pub(crate) fn clear_notes(&mut self) {
    self.error = None;
    self.reader.notice("");
  }

  /// Whether a press only dismisses something.
  pub(crate) fn overlaid(&self) -> bool {
    self.clues.shown().is_some() || self.input_owner().is_overlay()
  }

  pub(crate) fn dismiss_click(&mut self) -> Result<bool> {
    if !self.overlaid() {
      return Ok(false);
    }
    match self.input_owner() {
      InputOwner::Prompt => {
        self.cancel_input()?;
        return Ok(true);
      }
      InputOwner::Hints => self.hints = None,
      InputOwner::Peek => self.dismiss_overlay(),
      InputOwner::Region | InputOwner::Context => {
        self.dismiss_overlay();
        self.disarm();
        return Ok(true);
      }
      _ => {}
    }
    self.reader.cancel();
    self.disarm();
    Ok(true)
  }
}
