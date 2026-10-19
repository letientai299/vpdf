use super::{InputOwner, Targets, gesture::Gesture};
use crate::pointer::{Region, Wheel};
use crate::scrollbar::Bar;
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub enum Surface {
  Ignore,
  MenuDrag,
  MenuWheel,
  Panel,
  Clues(Wheel),
  PageWheel,
  Content(Content),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Content {
  Page,
  Overview,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub enum Dispatch {
  Ignore,
  Overview,
  Wheel,
  Bar,
  Alt,
  Left,
  Context,
  Middle,
}

impl Targets<'_> {
  /// Resolve after click dismissal and scrub release.
  pub fn surface(&self, kind: MouseEventKind, region: Region) -> Surface {
    if matches!(
      kind,
      MouseEventKind::Drag(MouseButton::Right)
        | MouseEventKind::Up(MouseButton::Right)
    ) && self.context.is_some()
    {
      return Surface::MenuDrag;
    }
    let wheel = Wheel::of(kind);
    if self.owner == InputOwner::Prompt
      || self.context.is_some() && wheel.is_none()
      || self.capture.is_some_and(crate::region::Region::is_bar)
    {
      return Surface::Ignore;
    }
    match region {
      Region::Status if wheel.is_some() => Surface::MenuWheel,
      Region::Panel => Surface::Panel,
      Region::Overlay => wheel.map_or(Surface::Ignore, Surface::Clues),
      Region::Status | Region::Actions | Region::Edge => Surface::Ignore,
      Region::Preview => Surface::PageWheel,
      Region::Menu => Surface::MenuWheel,
      Region::Page => Surface::Content(Content::Page),
      Region::Overview => Surface::Content(Content::Overview),
    }
  }
}

impl Content {
  /// Resolve after focus changes and relayout.
  pub fn dispatch(
    self,
    mouse: MouseEvent,
    overview: bool,
    bar: &Bar,
    gesture: &Gesture,
    on_bar: impl FnOnce() -> bool,
  ) -> Dispatch {
    if self == Self::Overview {
      return Dispatch::Overview;
    }
    if overview {
      return Dispatch::Ignore;
    }
    match mouse.kind {
      kind if Wheel::of(kind).is_some() => Dispatch::Wheel,
      MouseEventKind::Down(MouseButton::Left)
        if on_bar() && mouse.modifiers.is_empty() =>
      {
        Dispatch::Bar
      }
      MouseEventKind::Drag(MouseButton::Left)
      | MouseEventKind::Up(MouseButton::Left)
        if bar.scrubbing() =>
      {
        Dispatch::Bar
      }
      MouseEventKind::Down(MouseButton::Left)
        if mouse.modifiers.contains(KeyModifiers::ALT) =>
      {
        Dispatch::Alt
      }
      MouseEventKind::Drag(MouseButton::Left)
      | MouseEventKind::Up(MouseButton::Left)
        if gesture.alt() =>
      {
        Dispatch::Alt
      }
      MouseEventKind::Down(MouseButton::Left)
      | MouseEventKind::Drag(MouseButton::Left)
      | MouseEventKind::Up(MouseButton::Left) => Dispatch::Left,
      MouseEventKind::Down(MouseButton::Right) => Dispatch::Context,
      MouseEventKind::Down(MouseButton::Middle)
      | MouseEventKind::Drag(MouseButton::Middle)
      | MouseEventKind::Up(MouseButton::Middle) => Dispatch::Middle,
      _ => Dispatch::Ignore,
    }
  }
}

#[cfg(test)]
mod tests;
