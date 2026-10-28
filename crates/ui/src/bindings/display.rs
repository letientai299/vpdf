use super::{
  Bindings, Change, Context, Control, Map, Sequence, Target, actions,
};
use crate::keys::{self, Action};
use serde::{Serialize, Serializer};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Write;

impl Target {
  pub fn topic(&self) -> String {
    if let Self::Reader(Action::Scroll(amount)) = self {
      match *amount {
        0.5 => return "half_down".into(),
        -0.5 => return "half_up".into(),
        1.0 => return "page_down".into(),
        -1.0 => return "page_up".into(),
        _ => {}
      }
    }
    if let Self::Command(command) = self {
      return format!("command.execute.{}", topic_value(&json!(command)));
    }
    let value = self.value();
    let mut id =
      value["action"].as_str().unwrap_or("action.unknown").to_owned();
    if let Some(fields) = value.as_object() {
      for (key, value) in fields {
        if key == "action" {
          continue;
        }
        id.push('.');
        id.push_str(key);
        id.push('_');
        id.push_str(&topic_value(value));
      }
    }
    id
  }

  pub fn about(&self) -> String {
    if let Self::Command(command) = self {
      return format!("Command: {command}");
    }
    if let Self::Reader(action) = self
      && let Some((_, _, about)) = keys::BINDINGS
        .iter()
        .chain(keys::RECENT_BINDINGS)
        .find(|(_, bound, _)| bound == action)
    {
      return (*about).into();
    }
    let value = self.value();
    let action = value["action"].as_str().unwrap_or("action");
    let mut title = action.replace(['.', '_'], " ");
    if let Some(fields) = value.as_object() {
      for (key, value) in fields {
        if key != "action" {
          let _ = write!(
            title,
            "; {}: {}",
            key.replace('_', " "),
            value.as_str().map_or_else(|| value.to_string(), str::to_owned)
          );
        }
      }
    }
    title
  }

  pub fn value(&self) -> Value {
    match self {
      Self::Command(command) => json!({"command": command}),
      Self::Prompt(action) => serde_json::to_value(action).unwrap_or_default(),
      Self::Hint(action) => serde_json::to_value(action).unwrap_or_default(),
      Self::Control(control) => {
        serde_json::to_value(control).unwrap_or_default()
      }
      Self::TextObject(object) => {
        let object = named(actions::OBJECTS, object);
        json!({"action": "object.select", "object": object})
      }
      Self::Reader(action) => reader_value(*action),
    }
  }

  pub fn label(&self) -> String {
    self.value().to_string()
  }
}

fn topic_value(value: &Value) -> String {
  let value = value.as_str().map_or_else(|| value.to_string(), str::to_owned);
  value
    .bytes()
    .map(|byte| {
      if byte.is_ascii_alphanumeric() || byte == b'-' {
        char::from(byte).to_string()
      } else {
        format!("_{byte:02x}")
      }
    })
    .collect()
}

fn named<T: PartialEq>(
  names: &[(&'static str, T)],
  value: &T,
) -> Option<&'static str> {
  names.iter().find(|(_, item)| item == value).map(|(name, _)| *name)
}

fn reader_value(action: Action) -> Value {
  use actions::Argument as A;
  let Some((spec, argument)) = actions::describe(action) else {
    return Value::Null;
  };
  let mut value = match argument {
    A::None => json!({}),
    A::Motion(motion) => {
      json!({"motion": named(actions::MOTIONS, &motion)})
    }
    A::Object { kind, around } => {
      json!({"object": named(actions::OBJECTS, &kind), "around": around})
    }
    A::Number(amount) => json!({"amount": amount}),
    A::Integer(amount) => json!({"amount": amount}),
    A::Character(name) => json!({"name": name}),
    A::Mark { name, exact } => json!({"name": name, "exact": exact}),
    A::Shift { dx, dy } => json!({"dx": dx, "dy": dy}),
    A::Window(window) => {
      serde_json::to_value(super::window::WindowArg::from(window))
        .unwrap_or_default()
    }
  };
  value["action"] = spec.name.into();
  value
}

impl Serialize for Target {
  fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
    self.value().serialize(serializer)
  }
}

impl Serialize for Bindings {
  fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
    let defaults = Self::default();
    let mut contexts = BTreeMap::new();
    for &context in Context::ALL {
      let changes = self.map(context).changes_from(&defaults.map(context));
      let entries: BTreeMap<_, _> = changes
        .into_iter()
        .filter_map(|edit| {
          let value = match edit.change {
            Change::Bind(target) => target.value(),
            Change::Unbind => json!({"unbind": true}),
            Change::Invalid(_) => return None,
          };
          Some((edit.sequence.to_string(), value))
        })
        .collect();
      if !entries.is_empty() {
        contexts.insert(context.name(), entries);
      }
    }
    contexts.serialize(serializer)
  }
}

impl Map<Target> {
  fn preferred(&self, target: &Target) -> Option<String> {
    if target.cancels()
      && let Ok(escape) = Sequence::parse("<Esc>")
      && self.get(&escape) == Some(target)
    {
      return Some(escape.to_string());
    }
    if let Target::Reader(action) = target
      && let Some(key) = keys::key_for(*action)
      && let Ok(sequence) = Sequence::parse(&keys::notation(key))
      && self.get(&sequence) == Some(target)
    {
      return Some(sequence.to_string());
    }
    None
  }

  pub(crate) fn shortcut(&self, target: &Target) -> String {
    if let Some(key) = self.preferred(target) {
      return key;
    }
    if let Some((sequence, _)) = self
      .entries()
      .filter(|(_, value)| *value == target)
      .min_by_key(|(sequence, _)| {
        (
          sequence.keys().len(),
          sequence.to_string().len(),
          sequence.to_string(),
        )
      })
    {
      return sequence.to_string();
    }
    String::new()
  }
}

impl Bindings {
  pub fn shortcut(&self, context: Context, target: &Target) -> String {
    let map = self.map(context);
    if let Some(key) = map.preferred(target) {
      return key;
    }
    if *target == Target::Reader(Action::YankLine)
      && matches!(context, Context::Panel | Context::Recent)
    {
      let first = self.shortcut(context, &Target::Control(Control::Yank));
      let second =
        self.shortcut(Context::Operator, &Target::Reader(Action::Yank));
      if !first.is_empty() && !second.is_empty() {
        return first + &second;
      }
    }
    map.shortcut(target)
  }

  pub fn hint(&self, context: Context, target: &Target, about: &str) -> String {
    let shortcut = self.shortcut(context, target);
    if shortcut.is_empty() {
      return String::new();
    }
    format!("{shortcut} {about}")
  }

  pub fn resize_hints(&self) -> String {
    use crate::split::Direction;
    use crate::window::Window;
    [
      (Action::Window(Window::Extend(Direction::Left)), "left"),
      (Action::Window(Window::Extend(Direction::Down)), "down"),
      (Action::Window(Window::Extend(Direction::Up)), "up"),
      (Action::Window(Window::Extend(Direction::Right)), "right"),
      (Action::Cancel, "done"),
    ]
    .into_iter()
    .map(|(action, about)| {
      self.hint(Context::Resize, &Target::Reader(action), about)
    })
    .filter(|hint| !hint.is_empty())
    .collect::<Vec<_>>()
    .join("  ")
  }

  pub fn region_hint(&self, drawing: bool) -> String {
    use vpdf_reader::motion::Motion as M;
    if *self == Self::default() {
      return if drawing {
        crate::region::DRAW_HINT.into()
      } else {
        crate::region::BAR_HINT.into()
      };
    }
    let mut hints = Vec::new();
    let movement: Vec<_> = [M::Left, M::Down, M::Up, M::Right]
      .into_iter()
      .map(|motion| {
        self.shortcut(Context::Region, &Target::Reader(Action::Motion(motion)))
      })
      .filter(|key| !key.is_empty())
      .collect();
    if !movement.is_empty() {
      hints.push(format!(
        "{} {}",
        movement.join("/"),
        if drawing { "resize" } else { "choose" }
      ));
    }
    let shifts: Vec<_> = [(-1, 0), (0, 1), (0, -1), (1, 0)]
      .into_iter()
      .map(|(dx, dy)| {
        self.shortcut(
          Context::Region,
          &Target::Reader(Action::RegionShift(dx, dy)),
        )
      })
      .filter(|key| !key.is_empty())
      .collect();
    if !shifts.is_empty() {
      hints.push(format!("{} move", shifts.join("/")));
    }
    hints.push(self.hint(
      Context::Region,
      &Target::Reader(if drawing { Action::Swap } else { Action::RegionStart }),
      if drawing { "swap" } else { "edit" },
    ));
    hints.push(self.hint(
      Context::Region,
      &Target::Reader(Action::Follow),
      if drawing { "capture" } else { "run" },
    ));
    hints.push(self.hint(
      Context::Region,
      &Target::Reader(Action::Cancel),
      "cancel",
    ));
    hints
      .into_iter()
      .filter(|hint| !hint.is_empty())
      .collect::<Vec<_>>()
      .join("; ")
  }
}
