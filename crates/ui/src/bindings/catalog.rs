//! Configuration action families accepted by the binding parser.

use super::{Control, PromptAction, Target, actions};
use crate::keys::Action;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Group {
  Reader,
  Window,
  Grammar,
  Object,
  Prompt,
  Hint,
  Target,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
  Bool,
  Number { min: f32, max: f32 },
  Integer { min: i32, max: i32 },
  Letter,
  Motion,
  Object,
  Axis,
  Direction,
  Command,
}

impl Value {
  pub fn constraint(self) -> String {
    match self {
      Self::Bool => "`true` or `false`".into(),
      Self::Number { min, max } if min.is_infinite() && max.is_infinite() => {
        "finite number".into()
      }
      Self::Number { min, max } => format!("number from {min} to {max}"),
      Self::Integer { min, max } => format!("integer from {min} to {max}"),
      Self::Letter => "one letter, `a`-`z` or `A`-`Z`".into(),
      Self::Motion => choices(actions::MOTIONS.iter().map(|(name, _)| *name)),
      Self::Object => choices(actions::OBJECTS.iter().map(|(name, _)| *name)),
      Self::Axis => choices(["rows", "columns"]),
      Self::Direction => choices(["left", "down", "up", "right"]),
      Self::Command => "Ex command without the leading colon".into(),
    }
  }

  /// A well-typed value the parser rejects, if any.
  pub fn invalid(self) -> Option<String> {
    match self {
      Self::Bool | Self::Command => None,
      Self::Number { max, .. } if max.is_infinite() => Some("nan".into()),
      Self::Number { max, .. } => Some((max + 0.5).to_string()),
      Self::Integer { max, .. } => Some((max + 1).to_string()),
      Self::Letter => Some("\"1\"".into()),
      Self::Motion | Self::Object | Self::Axis | Self::Direction => {
        Some("\"unknown\"".into())
      }
    }
  }
}

fn choices<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
  let names: Vec<_> =
    names.into_iter().map(|name| format!("`{name}`")).collect();
  format!("one of {}", names.join(", "))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
  pub name: &'static str,
  pub value: Value,
}

/// One `operation` of the `window` family.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Variant {
  pub name: &'static str,
  pub purpose: &'static str,
  pub fields: &'static [Field],
  pub example: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Family {
  /// The `action` value, or the target key for `command` and `unbind`.
  pub name: &'static str,
  pub group: Group,
  pub purpose: String,
  pub fields: &'static [Field],
  /// TOML fields after the name, for example `amount = 0.5`.
  pub example: &'static str,
  pub variants: &'static [Variant],
}

impl Family {
  pub fn topic(&self) -> String {
    let kind = if self.group == Group::Target { "target" } else { "action" };
    format!("{kind}.{}", self.name)
  }

  /// TOML inline table for the example binding.
  pub fn table(&self) -> String {
    self.inline(self.example)
  }

  pub fn variant_table(&self, variant: &Variant) -> String {
    let fields = if variant.example.is_empty() {
      format!("operation = \"{}\"", variant.name)
    } else {
      format!("operation = \"{}\", {}", variant.name, variant.example)
    };
    self.inline(&fields)
  }

  fn inline(&self, fields: &str) -> String {
    let name = self.name;
    if self.group == Group::Target {
      format!("{{ {fields} }}")
    } else if fields.is_empty() {
      format!("{{ action = \"{name}\" }}")
    } else {
      format!("{{ action = \"{name}\", {fields} }}")
    }
  }

  /// The catalog topic an instantiated target belongs to.
  pub fn topic_of(target: &Target) -> String {
    match target {
      Target::Command(_) => "target.command".into(),
      Target::TextObject(_) => "action.object.select".into(),
      _ => format!(
        "action.{}",
        target.value()["action"].as_str().unwrap_or_default()
      ),
    }
  }
}

const fn field(name: &'static str, value: Value) -> Field {
  Field { name, value }
}

const BOOL: Value = Value::Bool;
const AMOUNT: Value = Value::Integer { min: -10_000, max: 10_000 };

struct Meta {
  name: &'static str,
  purpose: &'static str,
  fields: &'static [Field],
  example: &'static str,
}

const fn meta(
  name: &'static str,
  purpose: &'static str,
  fields: &'static [Field],
  example: &'static str,
) -> Meta {
  Meta { name, purpose, fields, example }
}

const PARAMETERS: &[Meta] = &[
  meta(
    "motion",
    "Move the cursor or selection by a named motion.",
    &[field("motion", Value::Motion)],
    "motion = \"next-page\"",
  ),
  meta(
    "object",
    "Select a text object directly.",
    &[field("object", Value::Object), field("around", BOOL)],
    "object = \"paragraph\", around = false",
  ),
  meta(
    "scroll",
    "Scroll by a fraction of the view height; negative scrolls up.",
    &[field(
      "amount",
      Value::Number { min: f32::NEG_INFINITY, max: f32::INFINITY },
    )],
    "amount = 0.5",
  ),
  meta(
    "screen",
    "Scroll by whole screens with overlap; negative scrolls up.",
    &[field("amount", AMOUNT)],
    "amount = 1",
  ),
  meta(
    "rows",
    "Scroll by text rows; negative scrolls up.",
    &[field("amount", AMOUNT)],
    "amount = 3",
  ),
  meta(
    "align",
    "Place the cursor line at a fraction of the view height.",
    &[field("amount", Value::Number { min: 0.0, max: 1.0 })],
    "amount = 0.5",
  ),
  meta(
    "zoom",
    "Change zoom by steps; negative zooms out.",
    &[field("amount", AMOUNT)],
    "amount = 1",
  ),
  meta(
    "mark.set",
    "Set one named mark at the cursor.",
    &[field("name", Value::Letter)],
    "name = \"a\"",
  ),
  meta(
    "mark.go",
    "Jump to one named mark; exact keeps the saved glyph.",
    &[field("name", Value::Letter), field("exact", BOOL)],
    "name = \"a\", exact = true",
  ),
  meta(
    "region.shift",
    "Move the region selection by cells.",
    &[
      field("dx", Value::Integer { min: -128, max: 127 }),
      field("dy", Value::Integer { min: -128, max: 127 }),
    ],
    "dx = 0, dy = 1",
  ),
  meta(
    "window",
    "Split, focus, resize, or move panes.",
    &[],
    "operation = \"split\", axis = \"columns\"",
  ),
  meta("prompt.start", "Move to the start of the line.", &[], ""),
  meta("prompt.end", "Move to the end of the line.", &[], ""),
  meta(
    "prompt.move",
    "Move one character.",
    &[field("forward", BOOL)],
    "forward = true",
  ),
  meta(
    "prompt.accept",
    "Insert the history suggestion shown after the cursor.",
    &[],
    "",
  ),
  meta(
    "prompt.word",
    "Move one word; shell words stop at punctuation.",
    &[field("forward", BOOL), field("shell", BOOL)],
    "forward = true, shell = false",
  ),
  meta(
    "prompt.delete",
    "Delete one character.",
    &[field("backward", BOOL)],
    "backward = true",
  ),
  meta(
    "prompt.delete-or-complete",
    "Delete the next character; at the line end, complete.",
    &[],
    "",
  ),
  meta(
    "prompt.kill",
    "Cut to the line end, or to the start with to_start.",
    &[field("to_start", BOOL)],
    "to_start = true",
  ),
  meta(
    "prompt.delete-word",
    "Delete one word.",
    &[field("forward", BOOL), field("shell", BOOL)],
    "forward = false, shell = true",
  ),
  meta("prompt.yank", "Paste the last cut text.", &[], ""),
  meta("prompt.transpose", "Swap the characters around the cursor.", &[], ""),
  meta(
    "prompt.history",
    "Recall history; search matches the typed text.",
    &[field("forward", BOOL), field("search", BOOL)],
    "forward = false, search = true",
  ),
  meta(
    "prompt.complete",
    "Cycle completions.",
    &[field("forward", BOOL)],
    "forward = true",
  ),
  meta("prompt.undo", "Undo the last edit.", &[], ""),
  meta("prompt.editor", "Edit the line in an external editor.", &[], ""),
  meta("prompt.submit", "Run the prompt.", &[], ""),
  meta("prompt.cancel", "Close the prompt.", &[], ""),
  meta(
    "hint.type",
    "Type one link hint label character.",
    &[field("character", Value::Letter)],
    "character = \"a\"",
  ),
  meta("hint.erase", "Erase the last typed label character.", &[], ""),
  meta("hint.follow", "Follow the selected hint.", &[], ""),
  meta("hint.close", "Close hints.", &[], ""),
  meta("hint.cancel", "Cancel hints and the pending operation.", &[], ""),
  meta(
    "count.digit",
    "Type one count digit.",
    &[field("digit", Value::Integer { min: 0, max: 9 })],
    "digit = 1",
  ),
  meta("register.choose", "Wait for a register name.", &[], ""),
  meta("mark.choose", "Wait for a mark name to set.", &[], ""),
  meta(
    "mark.jump",
    "Wait for a mark name to jump to.",
    &[field("exact", BOOL)],
    "exact = true",
  ),
  meta(
    "object.begin",
    "Wait for a text object; around includes surroundings.",
    &[field("around", BOOL)],
    "around = false",
  ),
  meta("window.resize", "Enter resize mode.", &[], ""),
  meta("yank.begin", "Start a pending yank.", &[], ""),
  meta(
    "object.select",
    "Choose the text object after object.begin.",
    &[field("object", Value::Object)],
    "object = \"word\"",
  ),
];

const TARGETS: &[Meta] = &[
  meta(
    "command",
    "Run an Ex command; no colon, no key replay.",
    &[field("command", Value::Command)],
    "command = \"config reload\"",
  ),
  meta(
    "unbind",
    "Remove a built-in or inherited mapping.",
    &[],
    "unbind = true",
  ),
];

const AXIS: &[Field] = &[field("axis", Value::Axis)];
const DIRECTION: &[Field] = &[field("direction", Value::Direction)];

const fn variant(
  name: &'static str,
  purpose: &'static str,
  fields: &'static [Field],
  example: &'static str,
) -> Variant {
  Variant { name, purpose, fields, example }
}

pub const WINDOW: &[Variant] = &[
  variant("split", "Split the pane.", AXIS, "axis = \"columns\""),
  variant("focus", "Focus a neighbor.", DIRECTION, "direction = \"left\""),
  variant("next", "Focus the next pane.", &[], ""),
  variant("previous", "Focus the previous pane.", &[], ""),
  variant("last", "Focus the last pane.", &[], ""),
  variant("close", "Close the pane.", &[], ""),
  variant("only", "Close other panes.", &[], ""),
  variant("equalize", "Equalize pane sizes.", &[], ""),
  variant(
    "resize",
    "Resize by cells.",
    &[field("axis", Value::Axis), field("amount", AMOUNT)],
    "axis = \"rows\", amount = 2",
  ),
  variant(
    "extend",
    "Move a border in resize mode.",
    DIRECTION,
    "direction = \"right\"",
  ),
  variant("maximize", "Maximize along an axis.", AXIS, "axis = \"rows\""),
  variant("exchange", "Swap with the next pane.", &[], ""),
  variant(
    "rotate",
    "Rotate panes.",
    &[field("forward", BOOL)],
    "forward = true",
  ),
  variant("new", "Split with Recent.", &[], ""),
  variant("move", "Move to an edge.", DIRECTION, "direction = \"left\""),
  variant("to-tab", "Move the pane to a new tab.", &[], ""),
];

fn group(name: &str) -> Group {
  match name {
    "window" | "window.resize" => Group::Window,
    "object" | "object.begin" | "object.select" => Group::Object,
    _ if name.starts_with("prompt.") => Group::Prompt,
    _ if name.starts_with("hint.") => Group::Hint,
    _ if Control::NAMES.contains(&name) => Group::Grammar,
    _ => Group::Reader,
  }
}

/// Every action name the binding parser accepts, plus target forms.
pub fn names() -> Vec<(Group, &'static str)> {
  let mut names: Vec<_> = actions::ACTIONS
    .iter()
    .map(|spec| spec.name)
    .chain(PromptAction::NAMES.iter().copied())
    .chain(super::HintAction::NAMES.iter().copied())
    .chain(Control::NAMES.iter().copied())
    .chain(["object.select"])
    .map(|name| (group(name), name))
    .collect();
  names.extend([(Group::Target, "command"), (Group::Target, "unbind")]);
  names
}

/// Fails when an accepted action lacks metadata.
pub fn families() -> crate::Result<Vec<Family>> {
  names()
    .into_iter()
    .map(|(group, name)| {
      let fixed = actions::lookup(name).and_then(|spec| match spec.kind {
        actions::Kind::Fixed(action) => Some(action),
        _ => None,
      });
      let row = if group == Group::Target { TARGETS } else { PARAMETERS }
        .iter()
        .find(|row| row.name == name);
      let (purpose, fields, example) = match (fixed, row) {
        (Some(action), None) => (about(action), &[][..], ""),
        (_, Some(row)) => (row.purpose.to_owned(), row.fields, row.example),
        (None, None) => anyhow::bail!("Missing catalog metadata: {name}"),
      };
      Ok(Family {
        name,
        group,
        purpose,
        fields,
        example,
        variants: if name == "window" { WINDOW } else { &[] },
      })
    })
    .collect()
}

fn about(action: Action) -> String {
  let about = Target::Reader(action).about();
  let mut chars = about.chars();
  chars.next().map_or_else(String::new, |first| {
    format!("{}{}.", first.to_uppercase(), chars.as_str())
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::bindings::HintAction;
  use crate::bindings::window::{AxisArg, DirectionArg, WindowArg};

  /// Exhaustive, so a new variant fails to compile until listed.
  const fn operation(arg: WindowArg) -> &'static str {
    use WindowArg as W;
    match arg {
      W::Split { .. } => "split",
      W::Focus { .. } => "focus",
      W::Next => "next",
      W::Previous => "previous",
      W::Last => "last",
      W::Close => "close",
      W::Only => "only",
      W::Equalize => "equalize",
      W::Resize { .. } => "resize",
      W::Extend { .. } => "extend",
      W::Maximize { .. } => "maximize",
      W::Exchange => "exchange",
      W::Rotate { .. } => "rotate",
      W::New => "new",
      W::Move { .. } => "move",
      W::ToTab => "to-tab",
    }
  }

  const fn hint(action: HintAction) -> HintAction {
    match action {
      HintAction::Type { .. }
      | HintAction::Erase
      | HintAction::Follow
      | HintAction::Close
      | HintAction::Cancel => action,
    }
  }

  #[test]
  fn hand_lists_cover_variants() {
    use WindowArg as W;
    let (axis, direction) = (AxisArg::Rows, DirectionArg::Left);
    let windows = [
      W::Split { axis },
      W::Focus { direction },
      W::Next,
      W::Previous,
      W::Last,
      W::Close,
      W::Only,
      W::Equalize,
      W::Resize { axis, amount: 1 },
      W::Extend { direction },
      W::Maximize { axis },
      W::Exchange,
      W::Rotate { forward: true },
      W::New,
      W::Move { direction },
      W::ToTab,
    ];
    assert_eq!(windows.len(), WINDOW.len());
    for arg in windows {
      let name = operation(arg);
      let value = serde_json::to_value(arg).unwrap();
      assert_eq!(value["operation"], name);
      assert!(WINDOW.iter().any(|variant| variant.name == name), "{name}");
    }
    let hints = [
      HintAction::Type { character: 'a' },
      HintAction::Erase,
      HintAction::Follow,
      HintAction::Close,
      HintAction::Cancel,
    ];
    assert_eq!(hints.len(), HintAction::NAMES.len());
    for action in hints {
      let value = serde_json::to_value(hint(action)).unwrap();
      let name = value["action"].as_str().unwrap();
      assert!(HintAction::NAMES.contains(&name), "{name}");
    }
  }
}
