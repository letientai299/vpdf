use crossterm::event::{
  KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
  MouseEventKind,
};
use eframe::egui::{self, Event, Key, Modifiers, PointerButton};
use vpdf::session::native::Input;
use vpdf_model::geometry::Screen;
use vpdf_model::style::Shape;

const WHEEL_TURNS: f32 = 64.0;

pub struct Preedit {
  pub text: String,
  pub active: Option<std::ops::Range<usize>>,
}

#[derive(Default)]
pub struct Adapter {
  held: Vec<MouseButton>,
  position: Option<egui::Pos2>,
  wheel: egui::Vec2,
  wheel_modifiers: Option<Modifiers>,
  screen: Option<Screen>,
  editable: bool,
  blurred: bool,
  preedit: Option<Preedit>,
  interrupt: bool,
}

impl Adapter {
  pub fn editable(&mut self, editable: bool) {
    self.editable = editable;
    if !editable {
      self.preedit = None;
    }
  }
  pub fn preedit(&self) -> Option<&Preedit> {
    self.preedit.as_ref()
  }
  pub fn interrupt(&mut self) -> bool {
    std::mem::take(&mut self.interrupt)
  }
  fn cancel(&mut self) {
    self.interrupt |= self.preedit.take().is_some();
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
        _ => {}
      }
    }
    output
  }
  fn keyboard(
    &mut self,
    event: &Event,
    modifiers: &mut Modifiers,
  ) -> Option<Vec<Input>> {
    let mut output = Vec::new();
    match event {
      Event::Text(text)
        if !modifiers.ctrl
          && !modifiers.mac_cmd
          && !modifiers.alt
          && self.preedit.is_none() =>
      {
        self.wheel = egui::Vec2::ZERO;
        output.extend(text.chars().filter(|ch| !ch.is_control()).map(|ch| {
          Input::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
        }));
      }
      Event::Ime(egui::ImeEvent::Preedit { text, active_range_chars })
        if self.editable && !self.blurred =>
      {
        self.preedit = (!text.is_empty()).then(|| Preedit {
          text: text.clone(),
          active: active_range_chars.clone(),
        });
      }
      Event::Ime(egui::ImeEvent::Commit(text))
        if self.editable && !self.blurred =>
      {
        self.preedit = None;
        if !text.is_empty() {
          output.push(Input::Paste(text.clone()));
        }
      }
      Event::Ime(egui::ImeEvent::DeleteSurrounding {
        before_chars,
        after_chars,
      }) if self.editable && !self.blurred => {
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
      Event::Paste(text) => {
        self.cancel();
        self.wheel = egui::Vec2::ZERO;
        output.push(Input::Paste(text.clone()));
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
      let kind = match (axis, turns.is_sign_positive()) {
        (0, true) => MouseEventKind::ScrollLeft,
        (0, false) => MouseEventKind::ScrollRight,
        (_, true) => MouseEventKind::ScrollUp,
        (_, false) => MouseEventKind::ScrollDown,
      };
      output.extend(
        std::iter::repeat_with(|| mouse(pos, kind, modifiers, screen, scale))
          .take(turns.abs().min(WHEEL_TURNS) as usize),
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
  let (x, y) = physical(pos, scale);
  Input::Mouse {
    event: MouseEvent {
      kind,
      column: x / screen.cell_width.max(1),
      row: y / screen.cell_height.max(1),
      modifiers: key_modifiers(modifiers),
    },
    pixels: (x, y),
    screen,
  }
}

fn physical(pos: egui::Pos2, scale: f32) -> (u16, u16) {
  (
    (pos.x * scale).floor().max(0.0) as u16,
    (pos.y * scale).floor().max(0.0) as u16,
  )
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

fn key_code(key: Key, modifiers: Modifiers) -> Option<KeyCode> {
  Some(match key {
    Key::ArrowDown => KeyCode::Down,
    Key::ArrowUp => KeyCode::Up,
    Key::ArrowLeft => KeyCode::Left,
    Key::ArrowRight => KeyCode::Right,
    Key::Escape => KeyCode::Esc,
    Key::Enter => KeyCode::Enter,
    Key::Tab => {
      if modifiers.shift {
        KeyCode::BackTab
      } else {
        KeyCode::Tab
      }
    }
    Key::Backspace => KeyCode::Backspace,
    Key::Delete => KeyCode::Delete,
    Key::Home => KeyCode::Home,
    Key::End => KeyCode::End,
    Key::PageUp => KeyCode::PageUp,
    Key::PageDown => KeyCode::PageDown,
    Key::F1 => KeyCode::F(1),
    Key::Space if modifiers.ctrl => KeyCode::Char(' '),
    _ if modifiers.ctrl || modifiers.mac_cmd || modifiers.alt => {
      let name = key.name();
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

#[cfg(test)]
mod tests {
  use super::*;

  fn events(events: &[Event], modifiers: Modifiers) -> Vec<Input> {
    Adapter::default().events(
      events,
      modifiers,
      screen(),
      1.0,
      [30.0, 60.0],
      None,
    )
  }

  fn screen() -> Screen {
    Screen {
      columns: 80,
      rows: 40,
      cell_width: 10,
      cell_height: 20,
      origin: (0, 0),
    }
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
  fn composition_commits_once_and_only_in_prompts() {
    let mut adapter = Adapter::default();
    let run = |adapter: &mut Adapter, input: &[Event]| {
      adapter.events(input, Modifiers::NONE, screen(), 1.0, [30.0, 60.0], None)
    };
    assert!(
      run(
        &mut adapter,
        &[preedit("章"), Event::Ime(egui::ImeEvent::Commit("章".into()))]
      )
      .is_empty()
    );
    adapter.editable(true);
    assert!(
      run(
        &mut adapter,
        &[
          preedit("章"),
          key(Key::Enter, Modifiers::NONE),
          Event::Text("章".into())
        ]
      )
      .is_empty()
    );
    assert_eq!(adapter.preedit().unwrap().text, "章");
    let output = run(
      &mut adapter,
      &[Event::Ime(egui::ImeEvent::Commit("章a\u{301}".into()))],
    );
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
      adapter.editable(true);
      let output = adapter.events(
        &[preedit("draft"), cancel],
        Modifiers::NONE,
        screen(),
        1.0,
        [30.0, 60.0],
        None,
      );
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
    adapter.editable(true);
    let output = adapter.events(
      &[
        preedit("draft"),
        Event::WindowFocused(false),
        Event::Ime(egui::ImeEvent::Commit("draft".into())),
      ],
      Modifiers::NONE,
      screen(),
      1.0,
      [30.0, 60.0],
      None,
    );
    assert!(matches!(&output[..], [Input::Focus(false)]));
    assert!(adapter.preedit().is_none());
    assert!(adapter.interrupt());
    assert!(!adapter.interrupt());
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
    let first = adapter.events(
      &[pressed],
      Modifiers::NONE,
      screen(),
      2.0,
      [30.0, 60.0],
      None,
    );
    let Input::Mouse { event, pixels, .. } = first[0] else {
      panic!("Expected press")
    };
    assert_eq!(pixels, (27, 50));
    assert_eq!((event.column, event.row), (2, 2));
    assert_eq!(event.modifiers, KeyModifiers::ALT);
    let next = adapter.events(
      &[Event::PointerMoved(point)],
      Modifiers::SHIFT,
      screen(),
      2.0,
      [30.0, 60.0],
      None,
    );
    let Input::Mouse { event, .. } = next[0] else { panic!("Expected drag") };
    assert_eq!(event.kind, MouseEventKind::Drag(MouseButton::Left));
    assert_eq!(event.modifiers, KeyModifiers::SHIFT);
    let next = adapter.events(
      &[Event::WindowFocused(false), Event::PointerMoved(point)],
      Modifiers::NONE,
      screen(),
      2.0,
      [30.0, 60.0],
      None,
    );
    assert!(matches!(next[0], Input::Focus(false)));
    let Input::Mouse { event, .. } = next[1] else { panic!("Expected motion") };
    assert_eq!(event.kind, MouseEventKind::Moved);
  }

  #[test]
  fn releases_and_exit_end_only_held_buttons() {
    let mut adapter = Adapter::default();
    let point = egui::pos2(30.0, 60.0);
    let button = |button, pressed| Event::PointerButton {
      pos: point,
      button,
      pressed,
      modifiers: Modifiers::NONE,
    };
    let events = [
      button(PointerButton::Primary, true),
      button(PointerButton::Middle, true),
      button(PointerButton::Primary, false),
      Event::PointerMoved(point),
      Event::PointerGone,
      Event::PointerMoved(point),
    ];
    let output = adapter.events(
      &events,
      Modifiers::NONE,
      screen(),
      1.0,
      [30.0, 60.0],
      None,
    );
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
    let button = |button, pressed| Event::PointerButton {
      pos: point,
      button,
      pressed,
      modifiers: Modifiers::NONE,
    };
    let output = adapter.events(
      &[
        button(PointerButton::Extra1, true),
        button(PointerButton::Extra1, false),
        button(PointerButton::Extra2, true),
        button(PointerButton::Extra2, false),
        Event::PointerMoved(point),
      ],
      Modifiers::NONE,
      screen(),
      2.0,
      [30.0, 60.0],
      None,
    );
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
    assert!(
      adapter
        .events(
          std::slice::from_ref(&event),
          Modifiers::NONE,
          screen(),
          2.0,
          [30.0, 60.0],
          pos
        )
        .is_empty()
    );
    let output = adapter.events(
      &[event],
      Modifiers::NONE,
      screen(),
      2.0,
      [30.0, 60.0],
      pos,
    );
    let Input::Mouse { event, pixels, .. } = output[0] else {
      panic!("Expected wheel")
    };
    assert_eq!(event.kind, MouseEventKind::ScrollDown);
    assert_eq!(pixels, (27, 51));
    let modifiers = Modifiers { ctrl: true, ..Modifiers::NONE };
    let output = adapter.events(
      &[wheel(egui::vec2(-1.0, 1.0), egui::MouseWheelUnit::Line, modifiers)],
      Modifiers::NONE,
      screen(),
      2.0,
      [30.0, 60.0],
      pos,
    );
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
  fn wheel_remainders_stop_at_gesture_and_geometry_changes() {
    let mut adapter = Adapter::default();
    let pos = Some(egui::pos2(10.0, 20.0));
    let partial = wheel(
      egui::vec2(0.0, -0.75),
      egui::MouseWheelUnit::Line,
      Modifiers::NONE,
    );
    assert!(
      adapter
        .events(
          std::slice::from_ref(&partial),
          Modifiers::NONE,
          screen(),
          1.0,
          [30.0, 60.0],
          pos
        )
        .is_empty()
    );
    let resized = Screen { cell_height: 40, ..screen() };
    assert!(
      adapter
        .events(
          std::slice::from_ref(&partial),
          Modifiers::NONE,
          resized,
          1.0,
          [30.0, 120.0],
          pos
        )
        .is_empty()
    );
    let mut end =
      wheel(egui::Vec2::ZERO, egui::MouseWheelUnit::Line, Modifiers::NONE);
    let Event::MouseWheel { phase, .. } = &mut end else { unreachable!() };
    *phase = egui::TouchPhase::End;
    adapter.events(&[end], Modifiers::NONE, resized, 1.0, [30.0, 120.0], pos);
    assert!(
      adapter
        .events(&[partial], Modifiers::NONE, resized, 1.0, [30.0, 120.0], pos)
        .is_empty()
    );
  }

  #[test]
  fn page_units_and_large_bursts_stay_bounded() {
    let mut adapter = Adapter::default();
    let pos = Some(egui::pos2(10.0, 20.0));
    let output = adapter.events(
      &[wheel(
        egui::vec2(0.0, -1.0),
        egui::MouseWheelUnit::Page,
        Modifiers::NONE,
      )],
      Modifiers::NONE,
      screen(),
      1.0,
      [30.0, 60.0],
      pos,
    );
    assert_eq!(output.len(), (screen().height() / 60.0).trunc() as usize);
    let output = adapter.events(
      &[wheel(
        egui::vec2(f32::MAX, f32::NAN),
        egui::MouseWheelUnit::Line,
        Modifiers::NONE,
      )],
      Modifiers::NONE,
      screen(),
      1.0,
      [30.0, 60.0],
      pos,
    );
    assert!(!output.is_empty() && output.len() < 1000);
    assert!(
      adapter
        .events(
          &[wheel(
            egui::vec2(0.0, -10.0),
            egui::MouseWheelUnit::Line,
            Modifiers::NONE
          )],
          Modifiers::NONE,
          screen(),
          1.0,
          [0.0; 2],
          pos
        )
        .is_empty()
    );
  }
}
