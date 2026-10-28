use super::KeyModifiers;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseButton {
  Left,
  Right,
  Middle,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseEventKind {
  Down(MouseButton),
  Up(MouseButton),
  Drag(MouseButton),
  Moved,
  ScrollDown,
  ScrollUp,
  ScrollLeft,
  ScrollRight,
}
/// Pointer position in cell coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseEvent {
  pub kind: MouseEventKind,
  pub column: u16,
  pub row: u16,
  pub modifiers: KeyModifiers,
}

impl MouseEvent {
  pub const fn new(kind: MouseEventKind, column: u16, row: u16) -> Self {
    Self { kind, column, row, modifiers: KeyModifiers::NONE }
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mouse {
  pub kind: MouseKind,
  pub at: Spot,
  pub modifiers: KeyModifiers,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseKind {
  Press(Button),
  Release(Button),
  Drag(Button),
  Moved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Button(u8);

impl Button {
  /// # Panics
  /// Button number outside 1–11.
  pub const fn new(number: u8) -> Self {
    assert!(number >= 1 && number <= 11);
    Self(number)
  }

  pub const BACK: Self = Self(8);
  pub const FORWARD: Self = Self(9);
}

/// Screen coordinates before pane adaptation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Spot {
  Cell { column: u16, row: u16 },
  Pixel { x: u16, y: u16 },
}

impl From<MouseEvent> for Mouse {
  fn from(event: MouseEvent) -> Self {
    let button = |button| match button {
      MouseButton::Left => Button(1),
      MouseButton::Middle => Button(2),
      MouseButton::Right => Button(3),
    };
    let kind = match event.kind {
      MouseEventKind::Down(pressed) => MouseKind::Press(button(pressed)),
      MouseEventKind::Up(released) => MouseKind::Release(button(released)),
      MouseEventKind::Drag(held) => MouseKind::Drag(button(held)),
      MouseEventKind::Moved => MouseKind::Moved,
      MouseEventKind::ScrollUp => MouseKind::Press(Button(4)),
      MouseEventKind::ScrollDown => MouseKind::Press(Button(5)),
      MouseEventKind::ScrollLeft => MouseKind::Press(Button(6)),
      MouseEventKind::ScrollRight => MouseKind::Press(Button(7)),
    };
    Self {
      kind,
      at: Spot::Cell { column: event.column, row: event.row },
      modifiers: event.modifiers,
    }
  }
}

impl Mouse {
  pub fn cells(self, cell: (u16, u16)) -> Option<MouseEvent> {
    let (column, row) = self.cell(cell);
    let kind = self.action()?;
    Some(MouseEvent { kind, column, row, modifiers: self.modifiers })
  }

  pub fn action(self) -> Option<MouseEventKind> {
    let button = |button: Button| match button.0 {
      1 => Some(MouseButton::Left),
      2 => Some(MouseButton::Middle),
      3 => Some(MouseButton::Right),
      _ => None,
    };
    let kind = match self.kind {
      MouseKind::Press(Button(4)) => MouseEventKind::ScrollUp,
      MouseKind::Press(Button(5)) => MouseEventKind::ScrollDown,
      MouseKind::Press(Button(6)) => MouseEventKind::ScrollLeft,
      MouseKind::Press(Button(7)) => MouseEventKind::ScrollRight,
      MouseKind::Press(pressed) => MouseEventKind::Down(button(pressed)?),
      MouseKind::Release(released) => MouseEventKind::Up(button(released)?),
      MouseKind::Drag(held) => MouseEventKind::Drag(button(held)?),
      MouseKind::Moved => MouseEventKind::Moved,
    };
    Some(kind)
  }

  pub fn cell(self, cell: (u16, u16)) -> (u16, u16) {
    match self.at {
      Spot::Cell { column, row } => (column, row),
      Spot::Pixel { x, y } => (x / cell.0.max(1), y / cell.1.max(1)),
    }
  }

  pub fn history(self) -> Option<bool> {
    match self.kind {
      MouseKind::Press(Button::BACK) => Some(false),
      MouseKind::Press(Button::FORWARD) => Some(true),
      _ => None,
    }
  }

  pub fn pixel(self) -> Option<(u16, u16)> {
    match self.at {
      Spot::Pixel { x, y } => Some((x, y)),
      Spot::Cell { .. } => None,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  const NONE: KeyModifiers = KeyModifiers::NONE;
  #[test]
  fn cell_buttons_keep_actions() {
    let mouse = |kind, at| Mouse { kind, at, modifiers: NONE };
    let cell = Spot::Cell { column: 3, row: 4 };
    let cases = [
      (
        mouse(MouseKind::Press(Button(1)), cell),
        Some(MouseEventKind::Down(MouseButton::Left)),
      ),
      (
        mouse(MouseKind::Drag(Button(3)), cell),
        Some(MouseEventKind::Drag(MouseButton::Right)),
      ),
      (
        mouse(MouseKind::Press(Button(5)), cell),
        Some(MouseEventKind::ScrollDown),
      ),
      (
        mouse(MouseKind::Press(Button(6)), cell),
        Some(MouseEventKind::ScrollLeft),
      ),
      (mouse(MouseKind::Moved, cell), Some(MouseEventKind::Moved)),
      (mouse(MouseKind::Press(Button::BACK), cell), None),
      (
        mouse(MouseKind::Press(Button(1)), Spot::Pixel { x: 3, y: 4 }),
        Some(MouseEventKind::Down(MouseButton::Left)),
      ),
    ];
    for (mouse, want) in cases {
      let kind = mouse.cells((8, 16)).map(|event| event.kind);
      assert_eq!(kind, want, "{mouse:?}");
    }
  }

  #[test]
  fn history_buttons_keep_direction() {
    let mouse = |kind| Mouse {
      kind,
      at: Spot::Cell { column: 3, row: 4 },
      modifiers: NONE,
    };
    let cases = [
      (MouseKind::Press(Button::BACK), Some(false)),
      (MouseKind::Press(Button::FORWARD), Some(true)),
      (MouseKind::Release(Button::BACK), None),
      (MouseKind::Drag(Button::FORWARD), None),
      (MouseKind::Press(Button(10)), None),
      (MouseKind::Press(Button::new(1)), None),
    ];
    for (kind, want) in cases {
      assert_eq!(mouse(kind).history(), want, "{kind:?}");
    }
  }

  #[test]
  fn cell_events_round_trip() {
    use MouseEventKind::{
      Down, Drag, Moved, ScrollDown, ScrollLeft, ScrollRight, ScrollUp, Up,
    };
    let mut kinds = vec![Moved, ScrollUp, ScrollDown, ScrollLeft, ScrollRight];
    for button in [MouseButton::Left, MouseButton::Middle, MouseButton::Right] {
      kinds.extend([Down(button), Drag(button), Up(button)]);
    }
    for kind in kinds {
      let event = MouseEvent {
        kind,
        column: 17,
        row: 23,
        modifiers: KeyModifiers::ALT | KeyModifiers::SHIFT,
      };
      assert_eq!(Mouse::from(event).cells((8, 16)), Some(event));
    }
  }
}
