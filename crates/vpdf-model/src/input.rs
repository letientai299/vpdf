mod pointer;
pub use pointer::*;

macro_rules! flags {
  ($name:ident { $($flag:ident = $bit:expr),* $(,)? }) => {
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub struct $name(u8);
    impl $name {
      $(pub const $flag: Self = Self($bit);)*
      pub const NONE: Self = Self(0);
      pub const fn empty() -> Self { Self::NONE }
      pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
      }
      pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
      }
      pub const fn is_empty(self) -> bool { self.0 == 0 }
    }
    impl std::ops::BitOr for $name {
      type Output = Self;
      fn bitor(self, other: Self) -> Self { Self(self.0 | other.0) }
    }
    impl std::ops::BitOrAssign for $name {
      fn bitor_assign(&mut self, other: Self) { *self = *self | other; }
    }
    impl std::ops::Sub for $name {
      type Output = Self;
      fn sub(self, other: Self) -> Self { Self(self.0 & !other.0) }
    }
  };
}
flags!(KeyModifiers {SHIFT=1, CONTROL=2, ALT=4, SUPER=8, HYPER=16, META=32});
flags!(KeyEventState {KEYPAD=1, CAPS_LOCK=2, NUM_LOCK=4});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyCode {
  Backspace,
  Enter,
  Left,
  Right,
  Up,
  Down,
  Home,
  End,
  PageUp,
  PageDown,
  Tab,
  BackTab,
  Delete,
  Insert,
  F(u8),
  Char(char),
  Null,
  Esc,
  CapsLock,
  ScrollLock,
  NumLock,
  PrintScreen,
  Pause,
  Menu,
  KeypadBegin,
  Media(MediaKeyCode),
  Modifier(ModifierKeyCode),
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaKeyCode {
  Play,
  Pause,
  PlayPause,
  Reverse,
  Stop,
  FastForward,
  Rewind,
  TrackNext,
  TrackPrevious,
  Record,
  LowerVolume,
  RaiseVolume,
  MuteVolume,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModifierKeyCode {
  LeftShift,
  LeftControl,
  LeftAlt,
  LeftSuper,
  LeftHyper,
  LeftMeta,
  RightShift,
  RightControl,
  RightAlt,
  RightSuper,
  RightHyper,
  RightMeta,
  IsoLevel3Shift,
  IsoLevel5Shift,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyEventKind {
  Press,
  Repeat,
  Release,
}

#[derive(Clone, Copy, Debug, Eq)]
pub struct KeyEvent {
  pub code: KeyCode,
  pub modifiers: KeyModifiers,
  pub kind: KeyEventKind,
  pub state: KeyEventState,
}
impl KeyEvent {
  pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
    Self::new_with_kind(code, modifiers, KeyEventKind::Press)
  }
  pub const fn new_with_kind(
    code: KeyCode,
    modifiers: KeyModifiers,
    kind: KeyEventKind,
  ) -> Self {
    Self { code, modifiers, kind, state: KeyEventState::NONE }
  }
  fn identity(self) -> (KeyCode, KeyModifiers, KeyEventKind, KeyEventState) {
    let (mut code, mut modifiers) = (self.code, self.modifiers);
    if let KeyCode::Char(ch) = code {
      if ch.is_ascii_uppercase() {
        modifiers |= KeyModifiers::SHIFT;
      } else if modifiers.contains(KeyModifiers::SHIFT) {
        code = KeyCode::Char(ch.to_ascii_uppercase());
      }
    }
    (code, modifiers, self.kind, self.state)
  }
}
impl PartialEq for KeyEvent {
  fn eq(&self, other: &Self) -> bool {
    self.identity() == other.identity()
  }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
  Key(KeyEvent),
  Paste(String),
  Mouse(Mouse),
}
