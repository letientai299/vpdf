use crossterm::event as raw;
use vpdf_model::input as core;
use vpdf_terminal::decode as wire;

pub(crate) fn key(key: raw::KeyEvent) -> core::KeyEvent {
  let code = match key.code {
    raw::KeyCode::Backspace => core::KeyCode::Backspace,
    raw::KeyCode::Enter => core::KeyCode::Enter,
    raw::KeyCode::Left => core::KeyCode::Left,
    raw::KeyCode::Right => core::KeyCode::Right,
    raw::KeyCode::Up => core::KeyCode::Up,
    raw::KeyCode::Down => core::KeyCode::Down,
    raw::KeyCode::Home => core::KeyCode::Home,
    raw::KeyCode::End => core::KeyCode::End,
    raw::KeyCode::PageUp => core::KeyCode::PageUp,
    raw::KeyCode::PageDown => core::KeyCode::PageDown,
    raw::KeyCode::Tab => core::KeyCode::Tab,
    raw::KeyCode::BackTab => core::KeyCode::BackTab,
    raw::KeyCode::Delete => core::KeyCode::Delete,
    raw::KeyCode::Insert => core::KeyCode::Insert,
    raw::KeyCode::Null => core::KeyCode::Null,
    raw::KeyCode::Esc => core::KeyCode::Esc,
    raw::KeyCode::CapsLock => core::KeyCode::CapsLock,
    raw::KeyCode::ScrollLock => core::KeyCode::ScrollLock,
    raw::KeyCode::NumLock => core::KeyCode::NumLock,
    raw::KeyCode::PrintScreen => core::KeyCode::PrintScreen,
    raw::KeyCode::Pause => core::KeyCode::Pause,
    raw::KeyCode::Menu => core::KeyCode::Menu,
    raw::KeyCode::KeypadBegin => core::KeyCode::KeypadBegin,
    raw::KeyCode::F(n) => core::KeyCode::F(n),
    raw::KeyCode::Char(ch) => core::KeyCode::Char(ch),
    raw::KeyCode::Media(code) => core::KeyCode::Media(match code {
      raw::MediaKeyCode::Play => core::MediaKeyCode::Play,
      raw::MediaKeyCode::Pause => core::MediaKeyCode::Pause,
      raw::MediaKeyCode::PlayPause => core::MediaKeyCode::PlayPause,
      raw::MediaKeyCode::Reverse => core::MediaKeyCode::Reverse,
      raw::MediaKeyCode::Stop => core::MediaKeyCode::Stop,
      raw::MediaKeyCode::FastForward => core::MediaKeyCode::FastForward,
      raw::MediaKeyCode::Rewind => core::MediaKeyCode::Rewind,
      raw::MediaKeyCode::TrackNext => core::MediaKeyCode::TrackNext,
      raw::MediaKeyCode::TrackPrevious => core::MediaKeyCode::TrackPrevious,
      raw::MediaKeyCode::Record => core::MediaKeyCode::Record,
      raw::MediaKeyCode::LowerVolume => core::MediaKeyCode::LowerVolume,
      raw::MediaKeyCode::RaiseVolume => core::MediaKeyCode::RaiseVolume,
      raw::MediaKeyCode::MuteVolume => core::MediaKeyCode::MuteVolume,
    }),
    raw::KeyCode::Modifier(code) => core::KeyCode::Modifier(match code {
      raw::ModifierKeyCode::LeftShift => core::ModifierKeyCode::LeftShift,
      raw::ModifierKeyCode::LeftControl => core::ModifierKeyCode::LeftControl,
      raw::ModifierKeyCode::LeftAlt => core::ModifierKeyCode::LeftAlt,
      raw::ModifierKeyCode::LeftSuper => core::ModifierKeyCode::LeftSuper,
      raw::ModifierKeyCode::LeftHyper => core::ModifierKeyCode::LeftHyper,
      raw::ModifierKeyCode::LeftMeta => core::ModifierKeyCode::LeftMeta,
      raw::ModifierKeyCode::RightShift => core::ModifierKeyCode::RightShift,
      raw::ModifierKeyCode::RightControl => core::ModifierKeyCode::RightControl,
      raw::ModifierKeyCode::RightAlt => core::ModifierKeyCode::RightAlt,
      raw::ModifierKeyCode::RightSuper => core::ModifierKeyCode::RightSuper,
      raw::ModifierKeyCode::RightHyper => core::ModifierKeyCode::RightHyper,
      raw::ModifierKeyCode::RightMeta => core::ModifierKeyCode::RightMeta,
      raw::ModifierKeyCode::IsoLevel3Shift => {
        core::ModifierKeyCode::IsoLevel3Shift
      }
      raw::ModifierKeyCode::IsoLevel5Shift => {
        core::ModifierKeyCode::IsoLevel5Shift
      }
    }),
  };
  let kind = match key.kind {
    raw::KeyEventKind::Press => core::KeyEventKind::Press,
    raw::KeyEventKind::Repeat => core::KeyEventKind::Repeat,
    raw::KeyEventKind::Release => core::KeyEventKind::Release,
  };
  let mut state = core::KeyEventState::NONE;
  if key.state.contains(raw::KeyEventState::KEYPAD) {
    state |= core::KeyEventState::KEYPAD;
  }
  if key.state.contains(raw::KeyEventState::CAPS_LOCK) {
    state |= core::KeyEventState::CAPS_LOCK;
  }
  if key.state.contains(raw::KeyEventState::NUM_LOCK) {
    state |= core::KeyEventState::NUM_LOCK;
  }
  core::KeyEvent { code, kind, state, modifiers: modifiers(key.modifiers) }
}

fn modifiers(flags: raw::KeyModifiers) -> core::KeyModifiers {
  let mut out = core::KeyModifiers::NONE;
  if flags.contains(raw::KeyModifiers::SHIFT) {
    out |= core::KeyModifiers::SHIFT;
  }
  if flags.contains(raw::KeyModifiers::CONTROL) {
    out |= core::KeyModifiers::CONTROL;
  }
  if flags.contains(raw::KeyModifiers::ALT) {
    out |= core::KeyModifiers::ALT;
  }
  if flags.contains(raw::KeyModifiers::SUPER) {
    out |= core::KeyModifiers::SUPER;
  }
  if flags.contains(raw::KeyModifiers::HYPER) {
    out |= core::KeyModifiers::HYPER;
  }
  if flags.contains(raw::KeyModifiers::META) {
    out |= core::KeyModifiers::META;
  }
  out
}

pub(crate) fn mouse(mouse: wire::Mouse) -> core::Mouse {
  let button = |b: wire::Button| core::Button::new(b.number());
  let kind = match mouse.kind {
    wire::MouseKind::Press(b) => core::MouseKind::Press(button(b)),
    wire::MouseKind::Release(b) => core::MouseKind::Release(button(b)),
    wire::MouseKind::Drag(b) => core::MouseKind::Drag(button(b)),
    wire::MouseKind::Moved => core::MouseKind::Moved,
  };
  let at = match mouse.at {
    wire::Spot::Cell { column, row } => core::Spot::Cell { column, row },
    wire::Spot::Pixel { x, y } => core::Spot::Pixel { x, y },
  };
  core::Mouse { kind, at, modifiers: modifiers(mouse.modifiers) }
}

pub(crate) fn event(event: wire::Event) -> Option<core::Event> {
  Some(match event {
    wire::Event::Key(value) => core::Event::Key(key(value)),
    wire::Event::Mouse(value) => core::Event::Mouse(mouse(value)),
    wire::Event::Paste(text) => core::Event::Paste(text),
    _ => return None,
  })
}
