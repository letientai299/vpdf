//! Views drawn over a pane's content, and which of them takes input.

use crate::clues::Clues;
use crate::context::ContextMenu;
use crate::hints::Hints;
use crate::input::{Cancel, InputOwner, Overlay, Targets};
use crate::keys::Action;
use crate::layout::Screen;
use crate::overview::Overview;
use crate::panel::Panel;
use crate::pointer::List;
use crate::preview::{Origin, Preview};
use crate::prompt::Prompt;
use crate::region::Region;

/// Generates read, write, open, and close access to one overlay.
macro_rules! slot {
  ($field:ident, $mutable:ident, $open:ident, $close:ident, $kind:ty) => {
    pub fn $field(&self) -> Option<&$kind> {
      self.$field.as_ref()
    }

    pub fn $mutable(&mut self) -> Option<&mut $kind> {
      self.$field.as_mut()
    }

    pub fn $open(&mut self, value: $kind) {
      self.$field = Some(value);
    }

    pub fn $close(&mut self) -> Option<$kind> {
      self.$field.take()
    }
  };
}

#[derive(Default)]
pub struct Overlays {
  prompt: Option<Prompt>,
  hints: Option<Hints>,
  clues: Clues,
  pub(crate) preview: Option<Preview>,
  menu: Option<ContextMenu>,
  pub(crate) region: Option<Region>,
  overview: Option<Overview>,
}

impl Overlays {
  slot!(prompt, prompt_mut, open_prompt, close_prompt, Prompt);
  slot!(hints, hints_mut, open_hints, close_hints, Hints);
  slot!(preview, preview_mut, open_preview, close_preview, Preview);
  slot!(menu, menu_mut, open_menu, close_menu, ContextMenu);
  slot!(region, region_mut, open_region, close_region, Region);
  slot!(overview, overview_mut, open_overview, close_overview, Overview);

  pub fn clues(&self) -> &Clues {
    &self.clues
  }

  pub fn clues_mut(&mut self) -> &mut Clues {
    &mut self.clues
  }

  /// The view that takes keys, given whether the side panel has focus.
  pub fn owner(&self, panel_focused: bool) -> InputOwner {
    InputOwner::resolve(|owner| match owner {
      InputOwner::Prompt => self.prompt.is_some(),
      InputOwner::Hints => self.hints.is_some(),
      InputOwner::Region => self.region.as_ref().is_some_and(Region::active),
      InputOwner::Context => self.menu.is_some(),
      InputOwner::Peek => {
        self.preview.as_ref().is_some_and(|peek| peek.origin() == Origin::Key)
      }
      InputOwner::Panel => panel_focused,
      InputOwner::Overview => self.overview.is_some(),
      InputOwner::Content => true,
    })
  }

  /// What Esc ends for `owner`; `pending` keys or operator first.
  pub fn cancel(&self, owner: InputOwner, pending: bool) -> Cancel {
    owner.cancel(pending, |owner| match owner {
      InputOwner::Peek => self.preview.is_some(),
      InputOwner::Region => self.region.as_ref().is_some_and(Region::active),
      InputOwner::Overview => self.overview.is_some(),
      _ => false,
    })
  }

  /// Whether `owner` takes `action`.
  pub fn allows(
    &self,
    owner: InputOwner,
    action: Action,
    panel: Option<&Panel>,
  ) -> bool {
    match owner {
      InputOwner::Context => {
        self.menu.as_ref().is_some_and(|menu| menu.allows(action))
      }
      InputOwner::Peek => Preview::allows(action),
      InputOwner::Region => {
        self.region.as_ref().is_some_and(|region| region.allows(action))
      }
      InputOwner::Overview => Overview::allows(action),
      InputOwner::Panel => panel.is_some_and(|panel| panel.allows(action)),
      _ => true,
    }
  }

  /// Selects an item, as keys do. Picking a completion would rewrite
  /// the line, so only clicks run completions and clues.
  pub fn pick(&mut self, list: List, index: usize) {
    match list {
      List::Menu => {
        if let Some(menu) = &mut self.menu {
          menu.select(index);
        }
      }
      List::Actions => {
        if let Some(region) = &mut self.region {
          region.select(index);
        }
      }
      List::Hints => {
        if let Some(hints) = &mut self.hints {
          hints.select(index);
        }
      }
      List::Clues | List::Completions => {}
    }
  }

  /// Closes the menu, the preview, and the region; whether a region
  /// closed.
  pub fn dismiss(&mut self) -> bool {
    self.menu = None;
    self.preview = None;
    self.region.take().is_some()
  }

  /// Drops views placed for the old pane size.
  pub fn resized(&mut self) {
    self.hints = None;
    self.preview = None;
    self.menu = None;
  }

  /// Whether no overlay writes text over the page.
  pub fn quiet(&self) -> bool {
    self.overview.is_none()
      && self.hints.is_none()
      && self.preview.is_none()
      && self.menu.is_none()
      && self.region.is_none()
      && self.clues.shown().is_none()
      && self.prompt.is_none()
  }

  /// The lists sharing the rows above the status line.
  pub fn bottom(&self, screen: Screen) -> Overlay<'_> {
    Overlay::new(screen, self.prompt.as_ref(), &self.clues)
  }

  /// Borrowed components for pointer routing.
  pub fn targets<'a>(
    &'a self,
    screen: Screen,
    panel: Option<&'a Panel>,
  ) -> Targets<'a> {
    Targets {
      panel,
      hints: self.hints.as_ref(),
      preview: self.preview.as_ref(),
      context: self.menu.as_ref(),
      capture: self.region.as_ref(),
      bottom: self.bottom(screen),
      owner: self.owner(panel.is_some_and(Panel::focused)),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::prompt::PromptKind;
  use crate::region::{CaptureId, Snapshot};
  use vpdf_model::text::{Bounds, PageId};

  fn bar() -> Region {
    Region::open(Snapshot {
      id: CaptureId::new(1),
      page: PageId(0),
      bounds: Bounds { x0: 0.0, y0: 0.0, x1: 10.0, y1: 10.0 },
    })
  }

  #[test]
  fn owners_follow_precedence_and_dismissal() {
    let mut overlays = Overlays::default();
    assert!(overlays.quiet());
    assert_eq!(overlays.owner(false), InputOwner::Content);
    assert_eq!(overlays.owner(true), InputOwner::Panel);
    overlays.open_region(bar());
    assert_eq!(overlays.owner(true), InputOwner::Region);
    overlays.open_prompt(Prompt::new(PromptKind::Command));
    assert_eq!(overlays.owner(true), InputOwner::Prompt);
    assert!(!overlays.quiet());
    assert!(overlays.dismiss());
    assert!(!overlays.dismiss());
    assert!(overlays.prompt().is_some());
    overlays.resized();
    assert!(overlays.close_prompt().is_some());
    assert!(overlays.quiet());
  }
}
