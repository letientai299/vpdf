//! Scripted input for live checks without window focus.
//!
//! macOS drops posted pointer events for inactive apps, so a hidden
//! `--events` path (usually a FIFO) feeds one event per line:
//! `key SPEC...`, `resize W H`, `place X Y`, `mark NAME`, `quit`,
//! `move X Y`, `down|up X Y [left|right|middle|back|forward] [MODS]`,
//! `wheel DX DY [point|line|page] [MODS]`, `mods [MODS]`, `gone`,
//! `focus 0|1`, `preedit TEXT`, `commit TEXT`, `paste TEXT`, `drop PATH`.
//! Coordinates are window points; MODS joins ctrl, alt, shift, cmd with `+`.

use eframe::egui::{
  self, DroppedFile, Event, ImeEvent, Modifiers, MouseWheelUnit, PointerButton,
  RawInput, TouchPhase,
};
use std::io::BufRead;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};
use vpdf::scenario::Step;
use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug)]
struct Dropped(PathBuf);

impl DroppedFile for Dropped {
  fn path(&self) -> &Path {
    &self.0
  }
  fn bytes(&self) -> Result<Vec<u8>, String> {
    std::fs::read(&self.0).map_err(|error| error.to_string())
  }
}

pub enum Injected {
  Event(Event),
  Drop(PathBuf),
  Step(Step),
}

pub struct Injector {
  receiver: Receiver<Injected>,
  geometry: Option<(Geometry, Instant)>,
}

enum Geometry {
  Size(egui::Vec2),
  Position(egui::Pos2),
}

impl Injector {
  fn new(receiver: Receiver<Injected>) -> Self {
    Self { receiver, geometry: None }
  }

  fn ready(&mut self, ctx: &egui::Context, raw: &RawInput) -> bool {
    let Some((expected, deadline)) = &self.geometry else { return true };
    let viewport = raw.viewports.get(&raw.viewport_id);
    let ready = match expected {
      Geometry::Size(size) => viewport
        .and_then(|view| view.inner_rect)
        .is_some_and(|rect| (rect.size() - *size).length() < 1.0),
      Geometry::Position(position) => viewport
        .and_then(|view| view.outer_rect)
        .is_some_and(|rect| (rect.min - *position).length() < 1.0),
    };
    if ready {
      self.geometry = None;
      return true;
    }
    if Instant::now() >= *deadline {
      eprintln!("Injected geometry timed out");
      ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    } else {
      ctx.request_repaint_after(Duration::from_millis(10));
    }
    false
  }

  pub fn apply(
    &mut self,
    ctx: &egui::Context,
    raw: &mut RawInput,
  ) -> Option<String> {
    if !self.ready(ctx, raw) {
      return None;
    }
    let injected = self.receiver.try_recv().ok()?;
    if let Injected::Step(step) = &injected {
      let geometry = match step {
        Step::Resize(width, height) => {
          Some(Geometry::Size(egui::vec2(*width as f32, *height as f32)))
        }
        Step::Place(x, y) => {
          Some(Geometry::Position(egui::pos2(*x as f32, *y as f32)))
        }
        _ => None,
      };
      self.geometry = geometry
        .map(|geometry| (geometry, Instant::now() + Duration::from_secs(5)));
    }
    let mark = match injected {
      Injected::Event(event) => {
        if let Event::PointerButton { modifiers, .. }
        | Event::MouseWheel { modifiers, .. } = &event
        {
          raw.events.push(Event::ModifiersChanged(*modifiers));
        }
        raw.events.push(event);
        None
      }
      Injected::Step(step) => apply_step(step, ctx, raw),
      Injected::Drop(path) => {
        raw.dropped_files.push(Arc::new(Dropped(path)));
        None
      }
    };
    ctx.request_repaint();
    mark
  }
}

pub fn spawn(path: &Path, ctx: egui::Context) -> Injector {
  let (sender, receiver) = channel();
  let path = path.to_path_buf();
  let reader = move || {
    // A FIFO reopens for each writer; a file plays once.
    let fifo =
      std::fs::metadata(&path).is_ok_and(|meta| meta.file_type().is_fifo());
    loop {
      let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) => {
          eprintln!("Input events unavailable: {error}");
          return;
        }
      };
      let lines = std::io::BufReader::new(file).lines();
      if !forward(lines.map_while(Result::ok), &sender, &ctx) || !fifo {
        return;
      }
    }
  };
  if let Err(error) =
    std::thread::Builder::new().name("vpdf-events".into()).spawn(reader)
  {
    eprintln!("Input events unavailable: {error}");
  }
  Injector::new(receiver)
}

fn forward(
  lines: impl Iterator<Item = String>,
  sender: &Sender<Injected>,
  ctx: &egui::Context,
) -> bool {
  for line in lines {
    if line.trim().is_empty() || line.trim_start().starts_with('#') {
      continue;
    }
    let Some(event) = parse(&line) else {
      eprintln!("Unknown input event: {line}");
      continue;
    };
    if sender.send(event).is_err() {
      return false;
    }
    ctx.request_repaint();
  }
  true
}

fn apply_step(
  step: Step,
  ctx: &egui::Context,
  raw: &mut RawInput,
) -> Option<String> {
  match step {
    Step::Keys(keys) => {
      for key in keys {
        if let Some(events) = key_events(key) {
          raw.events.extend(events);
        }
      }
      raw.events.push(Event::ModifiersChanged(Modifiers::NONE));
    }
    Step::Resize(width, height) => ctx.send_viewport_cmd(
      egui::ViewportCommand::InnerSize(egui::vec2(width as f32, height as f32)),
    ),
    Step::Place(x, y) => ctx.send_viewport_cmd(
      egui::ViewportCommand::OuterPosition(egui::pos2(x as f32, y as f32)),
    ),
    Step::Mark(name) => return Some(name),
    Step::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
    _ => eprintln!("Scenario step requires driver"),
  }
  None
}

fn key_events(key: KeyEvent) -> Option<Vec<Event>> {
  let modifiers = Modifiers {
    ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
    alt: key.modifiers.contains(KeyModifiers::ALT),
    shift: key.modifiers.contains(KeyModifiers::SHIFT),
    mac_cmd: key.modifiers.contains(KeyModifiers::SUPER),
    command: key.modifiers.contains(KeyModifiers::SUPER),
  };
  let mut events = vec![Event::ModifiersChanged(modifiers)];
  if let KeyCode::Char(ch) = key.code
    && !modifiers.ctrl
    && !modifiers.alt
    && !modifiers.mac_cmd
  {
    let ch = if modifiers.shift { ch.to_ascii_uppercase() } else { ch };
    let key = egui::Key::from_name(&ch.to_string()).unwrap_or(egui::Key::Space);
    events.push(Event::Key {
      key,
      physical_key: None,
      pressed: true,
      repeat: false,
      modifiers,
    });
    events.push(Event::Text(ch.to_string()));
    events.push(Event::Key {
      key,
      physical_key: None,
      pressed: false,
      repeat: false,
      modifiers,
    });
    return Some(events);
  }
  let code = match key.code {
    KeyCode::Char(' ') => egui::Key::Space,
    KeyCode::Char('_') => egui::Key::Minus,
    KeyCode::Char(ch) => egui::Key::from_name(&ch.to_string())?,
    KeyCode::Enter => egui::Key::Enter,
    KeyCode::Esc => egui::Key::Escape,
    KeyCode::Tab | KeyCode::BackTab => egui::Key::Tab,
    KeyCode::Backspace => egui::Key::Backspace,
    KeyCode::Delete => egui::Key::Delete,
    KeyCode::Up => egui::Key::ArrowUp,
    KeyCode::Down => egui::Key::ArrowDown,
    KeyCode::Left => egui::Key::ArrowLeft,
    KeyCode::Right => egui::Key::ArrowRight,
    KeyCode::Home => egui::Key::Home,
    KeyCode::End => egui::Key::End,
    KeyCode::PageUp => egui::Key::PageUp,
    KeyCode::PageDown => egui::Key::PageDown,
    _ => return None,
  };
  events.push(Event::Key {
    key: code,
    physical_key: None,
    pressed: true,
    repeat: false,
    modifiers,
  });
  events.push(Event::Key {
    key: code,
    physical_key: None,
    pressed: false,
    repeat: false,
    modifiers,
  });
  Some(events)
}

fn parse(line: &str) -> Option<Injected> {
  let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
  if matches!(verb, "key" | "resize" | "place" | "mark" | "quit") {
    let step = Step::parse(line).ok().flatten()?;
    if let Step::Keys(keys) = &step
      && keys.iter().any(|key| key_events(*key).is_none())
    {
      return None;
    }
    return Some(Injected::Step(step));
  }
  let text = || rest.to_owned();
  let event = match verb {
    "drop" if !rest.is_empty() => return Some(Injected::Drop(rest.into())),
    "gone" => Event::PointerGone,
    "mods" => Event::ModifiersChanged(modifiers(rest)?),
    "focus" => Event::WindowFocused(rest == "1"),
    "paste" => Event::Paste(text()),
    "preedit" => {
      Event::Ime(ImeEvent::Preedit { text: text(), active_range_chars: None })
    }
    "commit" => Event::Ime(ImeEvent::Commit(text())),
    _ => return pointer(verb, rest).map(Injected::Event),
  };
  Some(Injected::Event(event))
}

fn pointer(verb: &str, rest: &str) -> Option<Event> {
  let words: Vec<_> = rest.split_whitespace().collect();
  let number = |index: usize| words.get(index)?.parse::<f32>().ok();
  let (x, y) = (number(0)?, number(1)?);
  let words = &words[2..];
  let modifiers = words.iter().find_map(|word| modifiers(word));
  let modifiers = modifiers.unwrap_or_default();
  Some(match verb {
    "move" => Event::PointerMoved(egui::pos2(x, y)),
    "down" | "up" => Event::PointerButton {
      pos: egui::pos2(x, y),
      button: match words.first().copied() {
        Some("right") => PointerButton::Secondary,
        Some("middle") => PointerButton::Middle,
        Some("back") => PointerButton::Extra1,
        Some("forward") => PointerButton::Extra2,
        _ => PointerButton::Primary,
      },
      pressed: verb == "down",
      modifiers,
    },
    "wheel" => Event::MouseWheel {
      delta: egui::vec2(x, y),
      unit: match words.first().copied() {
        Some("point") => MouseWheelUnit::Point,
        Some("page") => MouseWheelUnit::Page,
        _ => MouseWheelUnit::Line,
      },
      modifiers,
      phase: TouchPhase::Move,
      source: egui::MouseWheelSource::Unknown,
    },
    _ => return None,
  })
}

fn modifiers(word: &str) -> Option<Modifiers> {
  let mut modifiers = Modifiers::NONE;
  for name in word.split('+').filter(|name| !name.is_empty()) {
    match name {
      "ctrl" => modifiers.ctrl = true,
      "alt" => modifiers.alt = true,
      "shift" => modifiers.shift = true,
      "cmd" => {
        modifiers.mac_cmd = true;
        modifiers.command = true;
      }
      _ => return None,
    }
  }
  Some(modifiers)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn keys_reach_adapter() {
    use crate::input::Adapter;
    use vpdf::session::native::Input;
    use vpdf_model::geometry::Screen;
    let screen = Screen {
      columns: 100,
      rows: 40,
      cell_width: 10,
      cell_height: 20,
      origin: (0, 0),
    };
    for spec in [
      "C-d", "S-space", "g o", "S-j", "enter", "pageup", "C-S-a", "A-x", "中",
      "C-_", "S-tab",
    ] {
      let Step::Keys(keys) =
        Step::parse(&format!("key {spec}")).unwrap().unwrap()
      else {
        panic!("Expected keys")
      };
      let mut adapter = Adapter::default();
      let events: Vec<_> = keys
        .iter()
        .flat_map(|key| key_events(*key).expect("Supported key"))
        .collect();
      let applied = adapter.events(
        &events,
        Modifiers::NONE,
        screen,
        1.0,
        [10.0, 20.0],
        None,
      );
      let got: Vec<_> = applied
        .into_iter()
        .filter_map(
          |input| if let Input::Key(key) = input { Some(key) } else { None },
        )
        .collect();
      assert_eq!(got, keys, "{spec}");
    }
  }

  #[test]
  fn geometry_holds_later_marks() {
    for command in ["resize 1000 700", "place 2200 100"] {
      let (sender, receiver) = channel();
      let mut injector = Injector::new(receiver);
      let ctx = egui::Context::default();
      sender.send(parse(command).unwrap()).ok();
      sender.send(parse("mark after geometry").unwrap()).ok();
      let mut raw = RawInput::default();
      assert_eq!(injector.apply(&ctx, &mut raw), None);
      assert_eq!(injector.apply(&ctx, &mut raw), None);
      let viewport = raw.viewports.entry(raw.viewport_id).or_default();
      viewport.inner_rect = Some(egui::Rect::from_min_size(
        egui::pos2(2200.0, 100.0),
        egui::vec2(1000.0, 700.0),
      ));
      viewport.outer_rect = viewport.inner_rect;
      assert_eq!(
        injector.apply(&ctx, &mut raw).as_deref(),
        Some("after geometry")
      );
    }
  }

  #[test]
  fn fifo_steps_preserve_order() {
    for prior in ["key g o", "paste text", "drop /paper.pdf", "move 1 2"] {
      let (sender, receiver) = channel();
      let ctx = egui::Context::default();
      let mut injector = Injector::new(receiver);
      sender.send(parse(prior).unwrap()).ok();
      sender.send(parse("mark after input").unwrap()).ok();
      let mut raw = RawInput::default();
      assert_eq!(injector.apply(&ctx, &mut raw), None, "{prior}");
      let mut raw = RawInput::default();
      assert_eq!(
        injector.apply(&ctx, &mut raw).as_deref(),
        Some("after input"),
        "{prior}"
      );
    }
  }

  #[test]
  fn lines_become_events() {
    let alt = Modifiers { alt: true, ..Modifiers::NONE };
    let Some(Injected::Event(Event::PointerButton {
      pos,
      button,
      pressed,
      modifiers,
    })) = parse("down 10 20.5 right alt")
    else {
      panic!("Expected press")
    };
    assert_eq!(
      (pos, button, pressed, modifiers),
      (egui::pos2(10.0, 20.5), PointerButton::Secondary, true, alt)
    );
    assert!(matches!(
      parse("wheel 0 -3 page ctrl+shift"),
      Some(Injected::Event(Event::MouseWheel {
        unit: MouseWheelUnit::Page,
        modifiers: Modifiers { ctrl: true, shift: true, .. },
        ..
      }))
    ));
    assert!(matches!(parse("drop /a b.pdf"),
      Some(Injected::Drop(path)) if path == Path::new("/a b.pdf")));
    let commit = parse("commit 中文");
    assert!(matches!(commit,
      Some(Injected::Event(Event::Ime(ImeEvent::Commit(text))))
        if text == "中文"));
    assert!(matches!(
      parse("mods"),
      Some(Injected::Event(Event::ModifiersChanged(Modifiers::NONE)))
    ));
    for line in ["down 1", "jump 1 2", "move x 2", "drop", "mods none"] {
      assert!(parse(line).is_none(), "{line}");
    }
  }
}
