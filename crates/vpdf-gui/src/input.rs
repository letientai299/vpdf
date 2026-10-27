use eframe::egui::{self, Event, Key, Modifiers, PointerButton};
use vpdf::session::native::{Gesture, Input, Precise};
use vpdf_model::geometry::Screen;
use vpdf_model::input::{
  KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
  MouseEventKind,
};
use vpdf_model::style::Shape;

const WHEEL_TURNS: f32 = 64.0;

pub struct Preedit {
  pub text: String,
  pub active: Option<std::ops::Range<usize>>,
}

#[derive(Default)]
pub struct Adapter {
  menu: Vec<egui::KeyboardShortcut>,
  snapshot: std::sync::Arc<vpdf::session::native::commands::Snapshot>,
  held: Vec<MouseButton>,
  position: Option<egui::Pos2>,
  wheel: egui::Vec2,
  wheel_modifiers: Option<Modifiers>,
  screen: Option<Screen>,
  editing: Option<egui::IMEPurpose>,
  cancelled: bool,
  blurred: bool,
  preedit: Option<Preedit>,
  interrupt: bool,
}

impl Adapter {
  pub fn reserve(&mut self, shortcut: egui::KeyboardShortcut) {
    self.menu.push(shortcut);
  }
  pub fn snapshot(
    &mut self,
    snapshot: std::sync::Arc<vpdf::session::native::commands::Snapshot>,
  ) {
    self.snapshot = snapshot;
  }
  pub fn editing(&mut self, purpose: Option<egui::IMEPurpose>) {
    if self.editing != purpose {
      self.cancel();
      self.editing = purpose;
    }
  }
  pub fn preedit(&self) -> Option<&Preedit> {
    self.preedit.as_ref()
  }
  pub fn interrupt(&mut self) -> bool {
    std::mem::take(&mut self.interrupt)
  }
  fn cancel(&mut self) {
    let active = self.preedit.take().is_some();
    self.interrupt |= active;
    self.cancelled |= active;
  }
  pub fn events(
    &mut self,
    events: &[Event],
    modifiers: Modifiers,
    screen: Screen,
    scale: f32,
    step: [f32; 2],
    pointer: Option<egui::Pos2>,
  ) -> Vec<Input> {
    if self.screen != Some(screen) {
      self.screen = Some(screen);
      self.wheel = egui::Vec2::ZERO;
    }
    self.position = self.position.or(pointer);
    let mut output = Vec::new();
    let mut text_modifiers = modifiers;
    let mut pinch = None;
    for event in events {
      if let Some(events) = self.keyboard(event, &mut text_modifiers) {
        output.extend(events);
        continue;
      }
      match event {
        Event::PointerButton { pos, button, pressed, modifiers } => {
          if *pressed {
            self.cancel();
          }
          self.position = Some(*pos);
          self.wheel = egui::Vec2::ZERO;
          if *pressed
            && matches!(button, PointerButton::Extra1 | PointerButton::Extra2)
          {
            let pixels = physical(*pos, scale);
            output.push(Input::History {
              forward: *button == PointerButton::Extra2,
              pixels,
              screen,
            });
          }
          let button = match button {
            PointerButton::Primary => Some(MouseButton::Left),
            PointerButton::Secondary => Some(MouseButton::Right),
            PointerButton::Middle => Some(MouseButton::Middle),
            _ => None,
          };
          let Some(button) = button else { continue };
          self.held.retain(|held| *held != button);
          if *pressed {
            self.held.push(button);
          }
          let kind = if *pressed {
            MouseEventKind::Down(button)
          } else {
            MouseEventKind::Up(button)
          };
          output.push(mouse(*pos, kind, *modifiers, screen, scale));
        }
        Event::PointerMoved(pos) => {
          if self.position != Some(*pos) {
            self.wheel = egui::Vec2::ZERO;
          }
          self.position = Some(*pos);
          let kind = self.held.last().map_or(MouseEventKind::Moved, |button| {
            MouseEventKind::Drag(*button)
          });
          output.push(mouse(*pos, kind, modifiers, screen, scale));
        }
        Event::PointerGone => {
          self.position = None;
          self.wheel = egui::Vec2::ZERO;
          self.held.clear();
          output.push(Input::Leave);
        }
        Event::WindowFocused(focused) => {
          self.blurred = !focused;
          if !focused {
            self.cancel();
            self.wheel = egui::Vec2::ZERO;
            self.held.clear();
          }
          output.push(Input::Focus(*focused));
        }
        Event::MouseWheel { delta, unit, modifiers, phase, .. } => {
          output.extend(self.wheel(
            (*delta, *unit),
            *phase,
            *modifiers,
            screen,
            scale,
            step,
          ));
        }
        Event::Zoom(factor) if factor.is_finite() && *factor > 0.0 => {
          pinch = Some(pinch.unwrap_or(1.0) * factor);
        }
        _ => {}
      }
    }
    // One zoom a frame; the worker relayouts once.
    if let (Some(pos), Some(pinch)) = (self.position, pinch) {
      let (event, pixels) =
        report(pos, MouseEventKind::ScrollUp, Modifiers::NONE, screen, scale);
      let gesture = Gesture::Pinch(pinch);
      output.push(Input::Gesture { event, gesture, pixels, screen });
    }
    output
  }
  fn clipboard(&mut self, event: &Event) -> Option<Vec<Input>> {
    let mut output = Vec::new();
    match event {
      Event::Copy | Event::Cut => {
        use vpdf::session::native::commands::Action;
        let action =
          if matches!(event, Event::Copy) { Action::Copy } else { Action::Cut };
        if let Some(command) = self.snapshot.command(action) {
          output.push(Input::Action(command));
        }
      }
      Event::Paste(text) if !self.menu.is_empty() => {
        self.cancel();
        if let Some(command) =
          self.snapshot.command(vpdf::session::native::commands::Action::Paste)
        {
          output.push(Input::PasteTarget { command, text: text.clone() });
        }
      }
      Event::Paste(text) => {
        self.cancel();
        self.wheel = egui::Vec2::ZERO;
        output.push(Input::Paste(text.clone()));
      }
      _ => return None,
    }
    Some(output)
  }
  fn keyboard(
    &mut self,
    event: &Event,
    modifiers: &mut Modifiers,
  ) -> Option<Vec<Input>> {
    if let Some(output) = self.clipboard(event) {
      return Some(output);
    }
    let mut output = Vec::new();
    match event {
      Event::Text(text)
        if !modifiers.ctrl
          && !modifiers.mac_cmd
          && (!modifiers.alt
            || (cfg!(target_os = "macos") && self.editing.is_some()))
          && self.preedit.is_none() =>
      {
        self.wheel = egui::Vec2::ZERO;
        output.extend(text.chars().filter(|ch| !ch.is_control()).map(|ch| {
          let keys = if ch == ' ' && modifiers.shift {
            KeyModifiers::SHIFT
          } else {
            KeyModifiers::NONE
          };
          Input::Key(KeyEvent::new(KeyCode::Char(ch), keys))
        }));
      }
      Event::Ime(egui::ImeEvent::Preedit { text, active_range_chars })
        if self.editing.is_some() && !self.blurred =>
      {
        if !text.is_empty() {
          self.cancelled = false;
        }
        self.preedit = (!text.is_empty()).then(|| Preedit {
          text: text.clone(),
          active: active_range_chars.clone(),
        });
      }
      Event::Ime(egui::ImeEvent::Commit(text))
        if self.editing.is_some() && !self.blurred =>
      {
        self.preedit = None;
        if std::mem::take(&mut self.cancelled) {
          return Some(output);
        }
        if !text.is_empty() {
          output.push(Input::Paste(text.clone()));
        }
      }
      Event::Ime(egui::ImeEvent::DeleteSurrounding {
        before_chars,
        after_chars,
      }) if self.editing.is_some() && !self.blurred => {
        self.cancel();
        for (code, count) in
          [(KeyCode::Backspace, *before_chars), (KeyCode::Delete, *after_chars)]
        {
          output.extend(
            std::iter::repeat_with(|| {
              Input::Key(KeyEvent::new(code, KeyModifiers::NONE))
            })
            .take(count),
          );
        }
      }
      Event::Key { key, modifiers: keys, .. }
        if self.menu.iter().any(|shortcut| {
          shortcut.logical_key == *key && keys.matches_exact(shortcut.modifiers)
        }) =>
      {
        *modifiers = *keys;
      }
      Event::Key { key, pressed: true, repeat, modifiers: keys, .. } => {
        *modifiers = *keys;
        self.wheel = egui::Vec2::ZERO;
        if self.preedit.is_some() {
          if *key == Key::Escape {
            self.cancel();
          }
          return Some(output);
        }
        if cfg!(target_os = "macos")
          && self.editing.is_some()
          && keys.alt
          && !keys.ctrl
          && !keys.mac_cmd
          && key.symbol_or_name().len() == 1
        {
          return Some(output);
        }
        let Some(code) = key_code(*key, *keys) else { return Some(output) };
        let mut event = KeyEvent::new(code, key_modifiers(*keys));
        event.kind =
          if *repeat { KeyEventKind::Repeat } else { KeyEventKind::Press };
        output.push(Input::Key(event));
      }
      _ => return None,
    }
    Some(output)
  }
  fn wheel(
    &mut self,
    delta: (egui::Vec2, egui::MouseWheelUnit),
    phase: egui::TouchPhase,
    modifiers: Modifiers,
    screen: Screen,
    scale: f32,
    step: [f32; 2],
  ) -> Vec<Input> {
    let Some(pos) = self.position else { return Vec::new() };
    if phase == egui::TouchPhase::Cancel {
      self.wheel = egui::Vec2::ZERO;
      return Vec::new();
    }
    if step.iter().any(|value| !value.is_finite() || *value <= 0.0) {
      return Vec::new();
    }
    if self.wheel_modifiers != Some(modifiers)
      || phase == egui::TouchPhase::Start
    {
      self.wheel = egui::Vec2::ZERO;
      self.wheel_modifiers = Some(modifiers);
    }
    let (delta, unit) = delta;
    let steps = match unit {
      egui::MouseWheelUnit::Point => {
        delta * scale / egui::vec2(step[0], step[1])
      }
      egui::MouseWheelUnit::Line => delta,
      egui::MouseWheelUnit::Page => {
        delta * egui::vec2(screen.width() / step[0], screen.height() / step[1])
      }
    };
    let mut output = Vec::new();
    for (axis, delta) in [steps.x, steps.y].into_iter().enumerate() {
      if !delta.is_finite() {
        continue;
      }
      self.wheel[axis] += delta;
      let turns = self.wheel[axis].trunc();
      self.wheel[axis] -= turns;
      let kind = match (axis, delta.is_sign_positive()) {
        (0, true) => MouseEventKind::ScrollLeft,
        (0, false) => MouseEventKind::ScrollRight,
        (_, true) => MouseEventKind::ScrollUp,
        (_, false) => MouseEventKind::ScrollDown,
      };
      let whole = turns.abs().min(WHEEL_TURNS) as u16;
      // Trackpads move the page by fractions.
      if unit == egui::MouseWheelUnit::Point && delta != 0.0 {
        let (event, pixels) = report(pos, kind, modifiers, screen, scale);
        let gesture = Gesture::Turn(Precise { turns: delta.abs(), whole });
        output.push(Input::Gesture { event, gesture, pixels, screen });
        continue;
      }
      output.extend(
        std::iter::repeat_with(|| mouse(pos, kind, modifiers, screen, scale))
          .take(usize::from(whole)),
      );
    }
    if phase == egui::TouchPhase::End {
      self.wheel = egui::Vec2::ZERO;
    }
    output
  }
}

fn mouse(
  pos: egui::Pos2,
  kind: MouseEventKind,
  modifiers: Modifiers,
  screen: Screen,
  scale: f32,
) -> Input {
  let (event, pixels) = report(pos, kind, modifiers, screen, scale);
  Input::Mouse { event, pixels, screen }
}

fn report(
  pos: egui::Pos2,
  kind: MouseEventKind,
  modifiers: Modifiers,
  screen: Screen,
  scale: f32,
) -> (MouseEvent, (u16, u16)) {
  let (x, y) = physical(pos, scale);
  let y =
    y.min(screen.rows.saturating_mul(screen.cell_height).saturating_sub(1));
  let event = MouseEvent {
    kind,
    column: x / screen.cell_width.max(1),
    row: y / screen.cell_height.max(1),
    modifiers: key_modifiers(modifiers),
  };
  (event, (x, y))
}

fn physical(pos: egui::Pos2, scale: f32) -> (u16, u16) {
  (
    (pos.x * scale).floor().max(0.0) as u16,
    (pos.y * scale).floor().max(0.0) as u16,
  )
}

pub fn purpose(
  scene: &[vpdf_model::scene::Op<'_>],
) -> Option<egui::IMEPurpose> {
  use vpdf_model::scene::Op;
  let mut purpose = None;
  let mut visible = false;
  for op in scene {
    match op {
      Op::InputArea { masked, .. } => {
        purpose = Some(if *masked {
          egui::IMEPurpose::Password
        } else {
          egui::IMEPurpose::Normal
        });
      }
      Op::ShowCursor => visible = true,
      Op::HideCursor => visible = false,
      _ => {}
    }
  }
  purpose.filter(|_| visible)
}

pub fn cursor(shape: Shape) -> egui::CursorIcon {
  match shape {
    Shape::Default => egui::CursorIcon::Default,
    Shape::Pointer => egui::CursorIcon::PointingHand,
    Shape::Text => egui::CursorIcon::Text,
    Shape::Crosshair => egui::CursorIcon::Crosshair,
    Shape::Grabbing => egui::CursorIcon::Grabbing,
    Shape::ColResize => egui::CursorIcon::ResizeColumn,
    Shape::RowResize => egui::CursorIcon::ResizeRow,
  }
}

pub(crate) const KEYS: [(Key, KeyCode); 14] = [
  (Key::ArrowDown, KeyCode::Down),
  (Key::ArrowUp, KeyCode::Up),
  (Key::ArrowLeft, KeyCode::Left),
  (Key::ArrowRight, KeyCode::Right),
  (Key::Escape, KeyCode::Esc),
  (Key::Enter, KeyCode::Enter),
  (Key::Tab, KeyCode::Tab),
  (Key::Backspace, KeyCode::Backspace),
  (Key::Delete, KeyCode::Delete),
  (Key::Home, KeyCode::Home),
  (Key::End, KeyCode::End),
  (Key::PageUp, KeyCode::PageUp),
  (Key::PageDown, KeyCode::PageDown),
  (Key::F1, KeyCode::F(1)),
];

fn key_code(key: Key, modifiers: Modifiers) -> Option<KeyCode> {
  if key == Key::Tab && modifiers.shift {
    return Some(KeyCode::BackTab);
  }
  if let Some(&(_, code)) = KEYS.iter().find(|(named, _)| *named == key) {
    return Some(code);
  }
  Some(match key {
    Key::Space if modifiers.ctrl => KeyCode::Char(' '),
    _ if modifiers.ctrl || modifiers.mac_cmd || modifiers.alt => {
      let name = match key {
        Key::Minus if modifiers.shift => "_",
        Key::Minus => "-",
        _ => key.symbol_or_name(),
      };
      if name.len() != 1 {
        return None;
      }
      let mut ch = name.chars().next()?;
      if !modifiers.shift {
        ch = ch.to_ascii_lowercase();
      }
      KeyCode::Char(ch)
    }
    _ => return None,
  })
}

fn key_modifiers(modifiers: Modifiers) -> KeyModifiers {
  let mut keys = KeyModifiers::NONE;
  if modifiers.ctrl {
    keys |= KeyModifiers::CONTROL;
  }
  if modifiers.alt {
    keys |= KeyModifiers::ALT;
  }
  if modifiers.shift {
    keys |= KeyModifiers::SHIFT;
  }
  if modifiers.mac_cmd {
    keys |= KeyModifiers::SUPER;
  }
  keys
}

#[cfg(feature = "bench-events")]
pub(crate) fn egui_modifiers(keys: KeyModifiers) -> Modifiers {
  let command = keys.contains(KeyModifiers::SUPER);
  Modifiers {
    ctrl: keys.contains(KeyModifiers::CONTROL),
    alt: keys.contains(KeyModifiers::ALT),
    shift: keys.contains(KeyModifiers::SHIFT),
    mac_cmd: command,
    command,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const STEP: [f32; 2] = [30.0, 60.0];

  fn events(events: &[Event], modifiers: Modifiers) -> Vec<Input> {
    Adapter::default().events(events, modifiers, screen(), 1.0, STEP, None)
  }

  /// `events` without held modifiers on the default screen.
  pub(super) fn feed(
    adapter: &mut Adapter,
    events: &[Event],
    scale: f32,
    pointer: Option<egui::Pos2>,
  ) -> Vec<Input> {
    adapter.events(events, Modifiers::NONE, screen(), scale, STEP, pointer)
  }

  fn button(pos: egui::Pos2, button: PointerButton, pressed: bool) -> Event {
    Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE }
  }

  fn screen() -> Screen {
    crate::screen::pixels(80, 40, 10, 20)
  }

  fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
      key,
      physical_key: Some(key),
      pressed: true,
      repeat: false,
      modifiers,
    }
  }

  #[test]
  fn clipboard_events_and_reserved_keys_emit_once() {
    use vpdf::session::native::commands::{Action, Snapshot};
    let mut adapter = Adapter::default();
    adapter.snapshot(std::sync::Arc::new(Snapshot {
      commands: vec![Action::Copy, Action::Cut, Action::Paste],
      ..Snapshot::default()
    }));
    adapter.reserve(egui::KeyboardShortcut::new(Modifiers::MAC_CMD, Key::C));
    adapter.reserve(egui::KeyboardShortcut::new(Modifiers::MAC_CMD, Key::X));
    let input = feed(
      &mut adapter,
      &[
        key(Key::C, Modifiers::MAC_CMD),
        Event::Copy,
        key(Key::X, Modifiers::MAC_CMD),
        Event::Cut,
      ],
      1.0,
      None,
    );
    let [Input::Action(copy), Input::Action(cut)] = input.as_slice() else {
      panic!("Expected clipboard commands")
    };
    assert_eq!(copy.action, Action::Copy);
    assert_eq!(cut.action, Action::Cut);
    let input = feed(&mut adapter, &[Event::Paste("日本語".into())], 1.0, None);
    assert!(
      matches!(&input[..], [Input::PasteTarget { text, .. }] if text == "日本語")
    );
    adapter.snapshot(std::sync::Arc::new(Snapshot::default()));
    assert!(
      feed(
        &mut adapter,
        &[Event::Copy, Event::Cut, Event::Paste("secret".into())],
        1.0,
        None
      )
      .is_empty()
    );
  }

  #[test]
  fn printable_key_and_text_emit_once() {
    let input = events(
      &[key(Key::J, Modifiers::NONE), Event::Text("j".into())],
      Modifiers::NONE,
    );
    assert_eq!(input.len(), 1);
    let Input::Key(event) = &input[0] else { panic!("Expected key") };
    assert_eq!(event.code, KeyCode::Char('j'));
    assert_eq!(event.modifiers, KeyModifiers::NONE);
  }

  #[test]
  fn shifted_space_keeps_scroll_direction() {
    for (key_code, text, modifiers, expected) in [
      (Key::Space, " ", Modifiers::NONE, KeyModifiers::NONE),
      (Key::Space, " ", Modifiers::SHIFT, KeyModifiers::SHIFT),
      (Key::J, "J", Modifiers::SHIFT, KeyModifiers::NONE),
      (Key::Plus, "+", Modifiers::SHIFT, KeyModifiers::NONE),
    ] {
      let input = events(
        &[key(key_code, modifiers), Event::Text(text.into())],
        Modifiers::NONE,
      );
      assert_eq!(input.len(), 1, "{text:?}");
      let Input::Key(event) = &input[0] else { panic!("Expected key") };
      assert_eq!(event.code, KeyCode::Char(text.chars().next().unwrap()));
      assert_eq!(event.modifiers, expected, "{text:?}");
    }
  }

  #[test]
  fn control_chords_and_paste_stay_distinct() {
    let modifiers = Modifiers { ctrl: true, ..Modifiers::NONE };
    let input = events(
      &[
        key(Key::W, modifiers),
        Event::Text("w".into()),
        Event::Paste("hello".into()),
      ],
      modifiers,
    );
    assert_eq!(input.len(), 2);
    let Input::Key(event) = &input[0] else { panic!("Expected control chord") };
    assert_eq!(event.code, KeyCode::Char('w'));
    assert_eq!(event.modifiers, KeyModifiers::CONTROL);
    assert!(matches!(&input[1], Input::Paste(text) if text == "hello"));
  }

  #[test]
  fn punctuation_chords_reach_prompt() {
    for (key_code, shift, expected) in [
      (Key::Minus, true, '_'),
      (Key::Minus, false, '-'),
      (Key::Slash, false, '/'),
      (Key::OpenBracket, false, '['),
    ] {
      let modifiers = Modifiers { ctrl: true, shift, ..Modifiers::NONE };
      let output = events(&[key(key_code, modifiers)], Modifiers::NONE);
      assert_eq!(output.len(), 1, "{key_code:?}");
      let Input::Key(event) = &output[0] else { panic!("Expected chord") };
      assert_eq!(event.code, KeyCode::Char(expected));
      assert_eq!(event.modifiers, key_modifiers(modifiers));
    }
  }

  #[test]
  fn meta_help_and_repeat_keep_shared_keys() {
    let alt = Modifiers { alt: true, ..Modifiers::NONE };
    let output =
      events(&[key(Key::B, alt), Event::Text("b".into())], Modifiers::NONE);
    assert_eq!(output.len(), 1);
    let Input::Key(event) = &output[0] else { panic!("Expected meta key") };
    assert_eq!(event.code, KeyCode::Char('b'));
    assert_eq!(event.modifiers, KeyModifiers::ALT);
    let mut repeated = key(Key::ArrowLeft, Modifiers::NONE);
    if let Event::Key { repeat, .. } = &mut repeated {
      *repeat = true;
    }
    let output =
      events(&[key(Key::F1, Modifiers::NONE), repeated], Modifiers::NONE);
    assert!(
      matches!(&output[0], Input::Key(event) if event.code == KeyCode::F(1))
    );
    assert!(matches!(&output[1], Input::Key(event)
      if event.code == KeyCode::Left && event.kind == KeyEventKind::Repeat));
  }

  fn preedit(text: &str) -> Event {
    Event::Ime(egui::ImeEvent::Preedit {
      text: text.into(),
      active_range_chars: Some(0..1),
    })
  }
  #[test]
  fn purpose_changes_cancel_composition() {
    use egui::IMEPurpose::{Normal, Password};
    use vpdf_model::scene::Op;
    let mut adapter = Adapter::default();
    adapter.editing(Some(Password));
    feed(&mut adapter, &[preedit("secret")], 1.0, None);
    let scene =
      [Op::InputArea { column: 0, columns: 80, masked: false }, Op::ShowCursor];
    adapter.editing(purpose(&scene));
    assert!(adapter.preedit().is_none());
    assert!(adapter.interrupt());
    let commit = Event::Ime(egui::ImeEvent::Commit("secret".into()));
    assert_eq!(feed(&mut adapter, &[commit], 1.0, None).len(), 0);
    feed(&mut adapter, &[preedit("new")], 1.0, None);
    adapter.editing(Some(Normal));
    assert_eq!(adapter.preedit().unwrap().text, "new");
    let commit = Event::Ime(egui::ImeEvent::Commit("new".into()));
    let output = feed(&mut adapter, &[commit], 1.0, None);
    assert!(matches!(&output[..], [Input::Paste(text)] if text == "new"));
    assert_eq!(purpose(&[Op::ShowCursor]), None);
    assert_eq!(
      purpose(&[
        Op::InputArea { column: 0, columns: 80, masked: true },
        Op::ShowCursor,
        Op::HideCursor,
      ]),
      None
    );
  }

  #[test]
  fn composition_commits_once_and_only_in_prompts() {
    let mut adapter = Adapter::default();
    let commit = Event::Ime(egui::ImeEvent::Commit("章".into()));
    assert!(feed(&mut adapter, &[preedit("章"), commit], 1.0, None).is_empty());
    adapter.editing(Some(egui::IMEPurpose::Normal));
    let enter = key(Key::Enter, Modifiers::NONE);
    let typed = [preedit("章"), enter, Event::Text("章".into())];
    assert!(feed(&mut adapter, &typed, 1.0, None).is_empty());
    assert_eq!(adapter.preedit().unwrap().text, "章");
    let commit = Event::Ime(egui::ImeEvent::Commit("章a\u{301}".into()));
    let output = feed(&mut adapter, &[commit], 1.0, None);
    assert!(
      matches!(&output[..], [Input::Paste(text)] if text == "章a\u{301}")
    );
    assert!(adapter.preedit().is_none());
    assert!(!adapter.interrupt());
  }

  #[test]
  fn composition_cancels_without_editing() {
    for cancel in [
      key(Key::Escape, Modifiers::NONE),
      Event::WindowFocused(false),
      preedit(""),
    ] {
      let mut adapter = Adapter::default();
      adapter.editing(Some(egui::IMEPurpose::Normal));
      let output = feed(&mut adapter, &[preedit("draft"), cancel], 1.0, None);
      assert!(adapter.preedit().is_none());
      assert!(
        !output
          .iter()
          .any(|event| matches!(event, Input::Key(_) | Input::Paste(_)))
      );
    }
  }
  #[test]
  fn blurred_composition_cannot_commit() {
    let mut adapter = Adapter::default();
    adapter.editing(Some(egui::IMEPurpose::Normal));
    let events = [
      preedit("draft"),
      Event::WindowFocused(false),
      Event::WindowFocused(true),
      Event::Ime(egui::ImeEvent::Commit("draft".into())),
    ];
    let output = feed(&mut adapter, &events, 1.0, None);
    assert!(matches!(&output[..], [Input::Focus(false), Input::Focus(true)]));
    assert!(adapter.preedit().is_none());
    assert!(adapter.interrupt());
    assert!(!adapter.interrupt());
  }
  #[test]
  fn bottom_remainder_hits_status() {
    let screen = screen();
    for scale in [1.0, 1.25, 1.5, 2.0] {
      let pos = egui::pos2(5.0, (screen.height() + 5.0) / scale);
      let input =
        mouse(pos, MouseEventKind::Moved, Modifiers::NONE, screen, scale);
      let Input::Mouse { event, pixels, .. } = input else {
        panic!("Expected mouse")
      };
      assert_eq!(event.row, screen.rows - 1);
      let mut report = vpdf_model::input::Mouse::from(event);
      report.at = vpdf_model::input::Spot::Pixel { x: pixels.0, y: pixels.1 };
      assert_eq!(
        report.cell((screen.cell_width, screen.cell_height)).1,
        event.row
      );
    }
  }

  #[test]
  fn drags_cross_frames_and_release_on_focus_loss() {
    let mut adapter = Adapter::default();
    let point = egui::pos2(13.75, 25.25);
    let pressed = Event::PointerButton {
      pos: point,
      button: PointerButton::Primary,
      pressed: true,
      modifiers: Modifiers { alt: true, ..Modifiers::NONE },
    };
    let first = feed(&mut adapter, &[pressed], 2.0, None);
    let Input::Mouse { event, pixels, .. } = first[0] else {
      panic!("Expected press")
    };
    assert_eq!(pixels, (27, 50));
    assert_eq!((event.column, event.row), (2, 2));
    assert_eq!(event.modifiers, KeyModifiers::ALT);
    let moved = [Event::PointerMoved(point)];
    let next =
      adapter.events(&moved, Modifiers::SHIFT, screen(), 2.0, STEP, None);
    let Input::Mouse { event, .. } = next[0] else { panic!("Expected drag") };
    assert_eq!(event.kind, MouseEventKind::Drag(MouseButton::Left));
    assert_eq!(event.modifiers, KeyModifiers::SHIFT);
    let blurred = [Event::WindowFocused(false), Event::PointerMoved(point)];
    let next = feed(&mut adapter, &blurred, 2.0, None);
    assert!(matches!(next[0], Input::Focus(false)));
    let Input::Mouse { event, .. } = next[1] else { panic!("Expected motion") };
    assert_eq!(event.kind, MouseEventKind::Moved);
  }

  #[test]
  fn releases_and_exit_end_only_held_buttons() {
    let mut adapter = Adapter::default();
    let point = egui::pos2(30.0, 60.0);
    let events = [
      button(point, PointerButton::Primary, true),
      button(point, PointerButton::Middle, true),
      button(point, PointerButton::Primary, false),
      Event::PointerMoved(point),
      Event::PointerGone,
      Event::PointerMoved(point),
    ];
    let output = feed(&mut adapter, &events, 1.0, None);
    let Input::Mouse { event, .. } = output[3] else {
      panic!("Expected middle drag")
    };
    assert_eq!(event.kind, MouseEventKind::Drag(MouseButton::Middle));
    assert!(matches!(output[4], Input::Leave));
    let Input::Mouse { event, .. } = output[5] else {
      panic!("Expected motion")
    };
    assert_eq!(event.kind, MouseEventKind::Moved);
  }
  #[test]
  fn extra_buttons_route_history_without_holding() {
    let mut adapter = Adapter::default();
    let point = egui::pos2(10.5, 20.5);
    let events = [
      button(point, PointerButton::Extra1, true),
      button(point, PointerButton::Extra1, false),
      button(point, PointerButton::Extra2, true),
      button(point, PointerButton::Extra2, false),
      Event::PointerMoved(point),
    ];
    let output = feed(&mut adapter, &events, 2.0, None);
    assert!(matches!(
      output[0],
      Input::History { forward: false, pixels: (21, 41), .. }
    ));
    assert!(matches!(output[1], Input::History { forward: true, .. }));
    let Input::Mouse { event, .. } = output[2] else {
      panic!("Expected motion")
    };
    assert_eq!(event.kind, MouseEventKind::Moved);
  }
  fn wheel(
    delta: egui::Vec2,
    unit: egui::MouseWheelUnit,
    modifiers: Modifiers,
  ) -> Event {
    Event::MouseWheel {
      delta,
      unit,
      modifiers,
      phase: egui::TouchPhase::Move,
      source: egui::MouseWheelSource::Unknown,
    }
  }

  #[test]
  fn fractional_wheel_uses_physical_cells_and_event_modifiers() {
    let mut adapter = Adapter::default();
    let pos = Some(egui::pos2(13.5, 25.5));
    let event = wheel(
      egui::vec2(0.0, -15.0),
      egui::MouseWheelUnit::Point,
      Modifiers::NONE,
    );
    // Each half turn carries its fraction; the second completes a turn.
    let mut whole = Vec::new();
    for _ in 0..2 {
      let output = feed(&mut adapter, std::slice::from_ref(&event), 2.0, pos);
      let [Input::Gesture { event, gesture: Gesture::Turn(turn), pixels, .. }] =
        output[..]
      else {
        panic!("Expected one precise turn")
      };
      assert_eq!(event.kind, MouseEventKind::ScrollDown);
      assert_eq!(pixels, (27, 51));
      assert_eq!(turn.turns, 0.5);
      whole.push(turn.whole);
    }
    assert_eq!(whole, [0, 1]);
    let modifiers = Modifiers { ctrl: true, ..Modifiers::NONE };
    let line =
      wheel(egui::vec2(-1.0, 1.0), egui::MouseWheelUnit::Line, modifiers);
    let output = feed(&mut adapter, &[line], 2.0, pos);
    let kinds: Vec<_> = output
      .iter()
      .map(|input| {
        let Input::Mouse { event, .. } = input else {
          panic!("Expected wheel")
        };
        assert_eq!(event.modifiers, KeyModifiers::CONTROL);
        event.kind
      })
      .collect();
    assert_eq!(kinds, [MouseEventKind::ScrollRight, MouseEventKind::ScrollUp]);
  }

  #[test]
  fn pinches_coalesce_at_the_pointer() {
    let mut adapter = Adapter::default();
    let pos = Some(egui::pos2(10.0, 20.0));
    let events = [
      Event::Zoom(1.25),
      Event::Zoom(f32::NAN),
      Event::Zoom(0.0),
      Event::Zoom(2.0),
    ];
    let output = feed(&mut adapter, &events, 2.0, pos);
    let [Input::Gesture { event, gesture, pixels, .. }] = output[..] else {
      panic!("Expected one pinch")
    };
    assert_eq!(gesture, Gesture::Pinch(2.5));
    assert_eq!((event.kind, pixels), (MouseEventKind::ScrollUp, (20, 40)));
    let mut idle = Adapter::default();
    assert!(feed(&mut idle, &[Event::Zoom(1.5)], 2.0, None).is_empty());
  }

  #[test]
  fn wheel_remainders_stop_at_gesture_and_geometry_changes() {
    let mut adapter = Adapter::default();
    let pos = Some(egui::pos2(10.0, 20.0));
    let partial = wheel(
      egui::vec2(0.0, -0.75),
      egui::MouseWheelUnit::Line,
      Modifiers::NONE,
    );
    let partial = std::slice::from_ref(&partial);
    assert!(feed(&mut adapter, partial, 1.0, pos).is_empty());
    let resized = Screen { cell_height: 40, ..screen() };
    let mut resized_feed = |events: &[Event]| {
      adapter.events(events, Modifiers::NONE, resized, 1.0, [30.0, 120.0], pos)
    };
    assert!(resized_feed(partial).is_empty());
    let mut end =
      wheel(egui::Vec2::ZERO, egui::MouseWheelUnit::Line, Modifiers::NONE);
    let Event::MouseWheel { phase, .. } = &mut end else { unreachable!() };
    *phase = egui::TouchPhase::End;
    resized_feed(&[end]);
    assert!(resized_feed(partial).is_empty());
  }

  #[test]
  fn page_units_and_large_bursts_stay_bounded() {
    let mut adapter = Adapter::default();
    let pos = Some(egui::pos2(10.0, 20.0));
    let line =
      |delta| wheel(delta, egui::MouseWheelUnit::Line, Modifiers::NONE);
    let page =
      wheel(egui::vec2(0.0, -1.0), egui::MouseWheelUnit::Page, Modifiers::NONE);
    let output = feed(&mut adapter, &[page], 1.0, pos);
    assert_eq!(output.len(), (screen().height() / 60.0).trunc() as usize);
    let burst = line(egui::vec2(f32::MAX, f32::NAN));
    let output = feed(&mut adapter, &[burst], 1.0, pos);
    assert!(!output.is_empty() && output.len() < 1000);
    let lines = [line(egui::vec2(0.0, -10.0))];
    let none = Modifiers::NONE;
    assert!(
      adapter.events(&lines, none, screen(), 1.0, [0.0; 2], pos).is_empty()
    );
  }
}

#[cfg(test)]
mod integration {
  use super::tests::feed;
  use super::*;

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn native_inputs_change_state() -> vpdf::Result<()> {
    let path =
      std::path::PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
    let screen = crate::screen::pixels(100, 40, 10, 20);
    let mut app = vpdf::app::App::with_screen(&path, "", true, screen)?;
    let mut adapter = Adapter::default();
    let mut history = vpdf::session::Session::new(None).history;
    for (text, expected) in [("G", 14), ("gg", 0)] {
      let output = feed(&mut adapter, &[Event::Text(text.into())], 1.0, None);
      assert_eq!(output.len(), text.len());
      for (input, ch) in output.into_iter().zip(text.chars()) {
        let Input::Key(key) = input else { panic!("key") };
        assert_eq!(key.code, KeyCode::Char(ch));
        app.key(key, &mut history)?;
      }
      assert_eq!(
        app.navigation_position().page,
        vpdf_model::text::PageId(expected)
      );
    }
    let alt = Modifiers { alt: true, ..Modifiers::NONE };
    let editing = [
      Event::Text(":".into()),
      Event::Paste("alpha beta".into()),
      Event::Key {
        key: Key::B,
        physical_key: Some(Key::B),
        pressed: true,
        repeat: false,
        modifiers: alt,
      },
    ];
    for input in feed(&mut adapter, &editing, 1.0, None) {
      match input {
        Input::Key(key) => app.key(key, &mut history)?,
        Input::Paste(text) => app.paste(&text, &mut history)?,
        _ => panic!("editing"),
      }
    }
    adapter.editing(Some(egui::IMEPurpose::Normal));
    let commit = Event::Ime(egui::ImeEvent::Commit("日本語".into()));
    let output = feed(&mut adapter, &[commit], 1.0, None);
    let [Input::Paste(text)] = output.as_slice() else { panic!("commit") };
    assert_eq!(text, "日本語");
    app.paste(text, &mut history)?;
    assert_eq!(app.overlays.prompt().unwrap().text(), "alpha 日本語beta");
    Ok(())
  }
  #[test]
  fn native_open_has_one_route() {
    let mut adapter = Adapter::default();
    let mut modifiers = Modifiers::MAC_CMD;
    let event = Event::Key {
      key: Key::O,
      physical_key: None,
      pressed: true,
      repeat: false,
      modifiers,
    };
    assert_eq!(adapter.keyboard(&event, &mut modifiers).unwrap().len(), 1);
    adapter.reserve(egui::KeyboardShortcut::new(Modifiers::MAC_CMD, Key::O));
    assert!(adapter.keyboard(&event, &mut modifiers).unwrap().is_empty());
    let event = Event::Key {
      key: Key::O,
      physical_key: None,
      pressed: true,
      repeat: false,
      modifiers: Modifiers::CTRL,
    };
    assert_eq!(adapter.keyboard(&event, &mut modifiers).unwrap().len(), 1);
  }
}
