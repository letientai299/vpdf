use super::{
  Context, Control, HintAction, Map, PromptAction, Sequence, Target,
};
use crate::keys::{self, Action};
use crate::split::Direction;
use crate::window::Window;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};
use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};
use vpdf_reader::motion::Motion;

#[derive(Clone, Debug, PartialEq)]
pub struct Bindings {
  maps: BTreeMap<Context, Arc<Map<Target>>>,
}

impl Default for Bindings {
  fn default() -> Self {
    static DEFAULT: OnceLock<Bindings> = OnceLock::new();
    DEFAULT
      .get_or_init(|| Self {
        maps: Context::ALL
          .iter()
          .map(|&context| (context, Arc::new(default_map(context))))
          .collect(),
      })
      .clone()
  }
}

impl Bindings {
  pub fn map(&self, context: Context) -> Arc<Map<Target>> {
    Arc::clone(&self.maps[&context])
  }

  pub fn set(&mut self, context: Context, map: Map<Target>) {
    self.maps.insert(context, Arc::new(map));
  }

  pub fn remember(&mut self, previous: &Self) {
    let customized = *previous != Self::default();
    for (&context, map) in &mut self.maps {
      Arc::make_mut(map).remember(&previous.map(context));
      if customized {
        Arc::make_mut(map).recognize(
          keys::Keys::references(context, previous)
            .into_iter()
            .map(|row| row.sequence),
        );
      }
    }
  }
}

fn default_map(context: Context) -> Map<Target> {
  use Context as C;
  if matches!(context, C::Recent | C::HomeRecent) {
    let mut map = default_map(context.parent().expect("recent parent"));
    for &(key, action, _) in keys::RECENT_BINDINGS {
      map.replace(
        &Sequence::parse(&keys::notation(key)).expect("recent key"),
        Some(&Target::Reader(action)),
      );
    }
    return map;
  }
  if matches!(context, C::Prompt | C::Command | C::Search | C::Password) {
    return Map::new(
      PromptAction::defaults(context == C::Password)
        .entries()
        .map(|(sequence, action)| (sequence.clone(), Target::Prompt(*action)))
        .collect(),
    )
    .expect("invalid prompt map");
  }
  let mut entries = Vec::new();
  if context == C::Hints {
    return hint_map();
  }
  if context == C::Object {
    for &(key, action, _) in keys::BINDINGS {
      if let Action::Object(object, false) = action {
        add(
          &mut entries,
          &keys::notation(&key[1..]),
          Target::TextObject(object),
        );
      }
    }
  } else if context == C::Resize {
    for (key, direction) in [
      ("h", Direction::Left),
      ("j", Direction::Down),
      ("k", Direction::Up),
      ("l", Direction::Right),
      ("<Left>", Direction::Left),
      ("<Down>", Direction::Down),
      ("<Up>", Direction::Up),
      ("<Right>", Direction::Right),
    ] {
      add(
        &mut entries,
        key,
        Target::Reader(Action::Window(Window::Extend(direction))),
      );
    }
  } else if context == C::MarkJump {
    for key in ["'", "`"] {
      add(&mut entries, key, Target::Reader(Action::History(false)));
    }
  } else if !matches!(context, C::Register | C::MarkSet) {
    reader_defaults(context, &mut entries);
  }

  if !matches!(context, C::Register | C::MarkSet | C::MarkJump) {
    for digit in 0..=9 {
      add(
        &mut entries,
        &digit.to_string(),
        Target::Control(Control::Digit { digit }),
      );
    }
  }
  for key in ["<Esc>", "<C-[>", "<C-c>"] {
    add(&mut entries, key, Target::Reader(Action::Cancel));
  }
  escape_aliases(&mut entries);
  shifted_aliases(&mut entries);
  entries.retain(|(_, target)| target.allowed(context));
  Map::new(entries).expect("invalid reader map")
}

fn hint_map() -> Map<Target> {
  let mut entries = Vec::new();
  for ch in 'a'..='z' {
    add(
      &mut entries,
      &ch.to_string(),
      Target::Hint(HintAction::Type { character: ch }),
    );
    add(
      &mut entries,
      &ch.to_ascii_uppercase().to_string(),
      Target::Hint(HintAction::Type { character: ch }),
    );
  }
  for (key, action) in [
    ("<BS>", HintAction::Erase),
    ("<CR>", HintAction::Follow),
    ("<C-m>", HintAction::Follow),
    ("<Esc>", HintAction::Cancel),
    ("<C-[>", HintAction::Cancel),
    ("<C-c>", HintAction::Cancel),
  ] {
    add(&mut entries, key, Target::Hint(action));
  }
  for modifiers in super::sequence::modifiers() {
    for (code, action) in [
      (KeyCode::Enter, HintAction::Follow),
      (
        KeyCode::Backspace,
        if (modifiers - KeyModifiers::SHIFT).is_empty() {
          HintAction::Erase
        } else {
          HintAction::Close
        },
      ),
      (KeyCode::Esc, HintAction::Cancel),
    ] {
      let key = KeyEvent::new(code, modifiers);
      let sequence = Sequence::from_keys(&[key]).expect("invalid hint alias");
      entries.retain(|(existing, _)| *existing != sequence);
      entries.push((sequence, Target::Hint(action)));
    }
  }
  for modifiers in super::sequence::modifiers() {
    for (code, action) in [
      (KeyCode::Char('m'), HintAction::Follow),
      (KeyCode::Char('['), HintAction::Cancel),
    ] {
      let key = KeyEvent::new(code, modifiers | KeyModifiers::CONTROL);
      let sequence = Sequence::from_keys(&[key]).expect("hint control alias");
      entries.retain(|(existing, _)| *existing != sequence);
      entries.push((sequence, Target::Hint(action)));
    }
  }
  Map::new(entries).expect("invalid hint map")
}

fn reader_defaults(context: Context, entries: &mut Vec<(Sequence, Target)>) {
  use Context as C;
  for &(key, action, _) in keys::BINDINGS {
    if key.starts_with("C-r")
      || key.starts_with('y') && key != "y"
      || matches!(action, Action::Object(..))
    {
      continue;
    }
    let target = if action == Action::Yank && context == C::Panel {
      Target::Control(Control::Yank)
    } else {
      Target::Reader(action)
    };
    add(entries, &keys::notation(key), target);
  }
  if context == C::Normal {
    add(entries, "i", Target::Reader(Action::Cursor));
  }
  if matches!(context, C::Visual | C::Operator) {
    add(entries, "i", Target::Control(Control::Object { around: false }));
    add(entries, "a", Target::Control(Control::Object { around: true }));
  }
  if context == C::Operator {
    for (key, action) in [
      ("y", Action::Yank),
      ("f", Action::Hints(true)),
      ("p", Action::YankPath),
      ("Y", Action::YankRaw),
    ] {
      add(entries, key, Target::Reader(action));
    }
  }
  if context == C::Region {
    for (key, dx, dy) in [("H", -1, 0), ("J", 0, 1), ("K", 0, -1), ("L", 1, 0)]
    {
      add(entries, key, Target::Reader(Action::RegionShift(dx, dy)));
    }
  }
  for (key, action) in [
    ("<Left>", Action::Motion(Motion::Left)),
    ("<Right>", Action::Motion(Motion::Right)),
    ("<Up>", Action::Motion(Motion::Up)),
    ("<Down>", Action::Motion(Motion::Down)),
    ("<C-w><Left>", Action::Window(Window::Focus(Direction::Left))),
    ("<C-w><Down>", Action::Window(Window::Focus(Direction::Down))),
    ("<C-w><Up>", Action::Window(Window::Focus(Direction::Up))),
    ("<C-w><Right>", Action::Window(Window::Focus(Direction::Right))),
    ("<PageUp>", Action::Scroll(-1.0)),
    ("<PageDown>", Action::Scroll(1.0)),
    ("<Home>", Action::Motion(Motion::First)),
    ("<End>", Action::Motion(Motion::Last)),
    ("<F1>", Action::Help),
    ("<S-Tab>", Action::LocalFocus),
    ("<C-m>", Action::Follow),
    ("<C-]>", Action::Follow),
  ] {
    add(entries, key, Target::Reader(action));
  }
  for (key, control) in [
    ("\"", Control::Register),
    ("m", Control::SetMark),
    ("'", Control::GoMark { exact: false }),
    ("`", Control::GoMark { exact: true }),
    ("<C-r>", Control::Resize),
  ] {
    add(entries, key, Target::Control(control));
  }
}

fn shifted_aliases(entries: &mut Vec<(Sequence, Target)>) {
  let originals = entries.clone();
  for (sequence, target) in originals {
    let mut variants = vec![sequence.keys().to_vec()];
    for index in 0..sequence.keys().len() {
      let key = sequence.keys()[index];
      if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(ch) if ch.is_ascii_lowercase())
      {
        let shifted = variants.clone().into_iter().map(|mut keys| {
          keys[index].modifiers |= KeyModifiers::SHIFT;
          keys
        });
        variants.extend(shifted);
      }
    }
    for keys in variants.into_iter().skip(1) {
      let sequence =
        Sequence::from_keys(&keys).expect("invalid shifted binding");
      if !entries.iter().any(|(key, _)| *key == sequence) {
        entries.push((sequence, target.clone()));
      }
    }
  }
}

fn escape_aliases(entries: &mut Vec<(Sequence, Target)>) {
  for modifiers in super::sequence::modifiers() {
    for key in [
      KeyEvent::new(KeyCode::Esc, modifiers),
      KeyEvent::new(KeyCode::Char('['), modifiers | KeyModifiers::CONTROL),
    ] {
      let sequence = Sequence::from_keys(&[key]).expect("invalid escape alias");
      entries.retain(|(existing, _)| *existing != sequence);
      entries.push((sequence, Target::Reader(Action::Cancel)));
    }
  }
}

fn add(entries: &mut Vec<(Sequence, Target)>, key: &str, target: Target) {
  let sequence = Sequence::parse(key).expect("invalid built-in binding");
  entries.retain(|(existing, _)| *existing != sequence);
  entries.push((sequence, target));
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn defaults_have_no_prefix_conflicts() {
    let bindings = Bindings::default();
    for &context in Context::ALL {
      assert!(bindings.map(context).entries().next().is_some());
      for (sequence, target) in bindings.map(context).entries() {
        assert!(
          target.allowed(context),
          "{}: {sequence}: {target:?}",
          context.name()
        );
      }
    }
  }
}
