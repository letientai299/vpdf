//! Scripted input for live checks without window focus.
//!
//! macOS drops posted pointer events for inactive apps, so a hidden
//! `--events` path (usually a FIFO) feeds one event per line:
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
}

pub fn spawn(path: &Path, ctx: egui::Context) -> Receiver<Injected> {
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
  receiver
}

fn forward(
  lines: impl Iterator<Item = String>,
  sender: &Sender<Injected>,
  ctx: &egui::Context,
) -> bool {
  for line in lines {
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

/// Ends the frame after a press or turn, so later moves see its modifiers.
pub fn apply(
  receiver: &Receiver<Injected>,
  ctx: &egui::Context,
  raw: &mut RawInput,
) {
  for injected in receiver.try_iter() {
    match injected {
      Injected::Event(event) => {
        let modifiers = match &event {
          Event::PointerButton { modifiers, .. }
          | Event::MouseWheel { modifiers, .. } => Some(*modifiers),
          _ => None,
        };
        if let Some(modifiers) = modifiers {
          raw.events.push(Event::ModifiersChanged(modifiers));
        }
        raw.events.push(event);
        if modifiers.is_some() {
          ctx.request_repaint();
          return;
        }
      }
      Injected::Drop(path) => raw.dropped_files.push(Arc::new(Dropped(path))),
    }
  }
}

fn parse(line: &str) -> Option<Injected> {
  let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
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
