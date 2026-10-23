use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;
use toml::Value;
use vpdf_ui::bindings::actions::{self, Argument, Kind};
use vpdf_ui::bindings::window::WindowArg;
use vpdf_ui::bindings::{
  Bindings, Change, Context, Control, Edit, HintAction, PromptAction, Sequence,
  Target,
};
use vpdf_ui::commands::{self, Id, Resolution, View};

pub(super) fn parse(
  value: &Value,
  previous: &Bindings,
  path: &Path,
  warnings: &mut Vec<String>,
) -> Bindings {
  let Some(table) = value.as_table() else {
    warnings.push("keybindings: expected table".into());
    return previous.clone();
  };
  let defaults = Bindings::default();
  let mut bindings = defaults.clone();
  let mut edits = BTreeMap::new();
  for (name, value) in table {
    let Some(context) = Context::parse(name) else {
      warnings.push(format!("keybindings.{name}: unknown context"));
      continue;
    };
    let Some(table) = value.as_table() else {
      warnings.push(format!("keybindings.{name}: expected table"));
      bindings.set(context, (*previous.map(context)).clone());
      continue;
    };
    let changes = table
      .iter()
      .filter_map(|(key, value)| {
        let sequence = match Sequence::parse(key) {
          Ok(sequence) => sequence,
          Err(error) => {
            warnings.push(format!("keybindings.{name}.{key}: {error}"));
            return None;
          }
        };
        let change = match target(value, context) {
          Ok(Some(target)) => Change::Bind(target),
          Ok(None) => Change::Unbind,
          Err(error) => Change::Invalid(error.to_string()),
        };
        Some(Edit { sequence, change })
      })
      .collect::<Vec<_>>();
    if context.parent().is_none() {
      let (map, diagnostics) = defaults.map(context).apply_sourced(
        &previous.map(context),
        &changes,
        |_| vpdf_ui::reference::Source::File(path.to_owned()),
      );
      report(context, diagnostics, warnings);
      bindings.set(context, map);
    }
    edits.insert(context, changes);
  }
  let mut common =
    bindings.map(Context::Prompt).changes_from(&defaults.map(Context::Prompt));
  for (sequence, target) in bindings.map(Context::Prompt).entries() {
    if bindings.map(Context::Prompt).source(sequence)
      != vpdf_ui::reference::Source::Default
      && !common.iter().any(|edit| edit.sequence == *sequence)
    {
      common.push(Edit {
        sequence: sequence.clone(),
        change: Change::Bind(target.clone()),
      });
    }
  }
  for context in [Context::Command, Context::Search, Context::Password] {
    if table.contains_key(context.name()) && !edits.contains_key(&context) {
      continue;
    }
    let mut inherited: Vec<_> = common.iter().filter(|edit| {
      !matches!(&edit.change, Change::Bind(target) if !target.allowed(context))
    }).cloned().collect();
    if let Some(own) = edits.get(&context) {
      inherited
        .retain(|edit| !own.iter().any(|own| own.sequence == edit.sequence));
      inherited.extend(own.iter().cloned());
    }
    let (map, diagnostics) = defaults.map(context).apply_sourced(
      &previous.map(context),
      &inherited,
      |sequence| {
        if edits
          .get(&context)
          .is_some_and(|own| own.iter().any(|edit| edit.sequence == *sequence))
        {
          vpdf_ui::reference::Source::File(path.to_owned())
        } else {
          bindings.map(Context::Prompt).source(sequence)
        }
      },
    );
    report(context, diagnostics, warnings);
    bindings.set(context, map);
  }
  bindings
}

fn report(
  context: Context,
  diagnostics: Vec<vpdf_ui::bindings::Diagnostic>,
  warnings: &mut Vec<String>,
) {
  for diagnostic in diagnostics {
    warnings.push(format!(
      "keybindings.{}.{}: {}",
      context.name(),
      diagnostic.sequence,
      diagnostic.reason
    ));
  }
}

fn target(value: &Value, context: Context) -> crate::Result<Option<Target>> {
  let table = value
    .as_table()
    .ok_or_else(|| anyhow::anyhow!("Expected binding target"))?;
  let discriminants = ["action", "command", "unbind"]
    .iter()
    .filter(|key| table.contains_key(**key))
    .count();
  anyhow::ensure!(discriminants == 1, "Expected one binding target");
  if let Some(unbind) = table.get("unbind") {
    anyhow::ensure!(
      table.len() == 1 && unbind.as_bool() == Some(true),
      "Expected unbind = true"
    );
    return Ok(None);
  }
  let target = if let Some(command) = table.get("command") {
    anyhow::ensure!(table.len() == 1, "Unexpected command argument");
    let command = command
      .as_str()
      .ok_or_else(|| anyhow::anyhow!("Expected command string"))?;
    let resolved = commands::resolve(
      command,
      if context == Context::Home { View::Home } else { View::Document },
      |_| true,
      |_| false,
    )?;
    let Resolution::Command { spec, argument } = resolved else {
      anyhow::bail!("Expected named command")
    };
    if spec.id == Id::Config {
      super::reload_command(argument)?;
    }
    if spec.id == Id::Theme {
      commands::theme(argument)?;
    }
    Target::Command(command.to_owned())
  } else {
    let name = table
      .get("action")
      .and_then(Value::as_str)
      .ok_or_else(|| anyhow::anyhow!("Expected action name"))?;
    if name.starts_with("prompt.") {
      Target::Prompt(value.clone().try_into::<PromptAction>()?)
    } else if name.starts_with("hint.") {
      Target::Hint(value.clone().try_into::<HintAction>()?)
    } else if Control::NAMES.contains(&name) {
      Target::Control(value.clone().try_into::<Control>()?)
    } else if name == "object.select" {
      let argument: TextObject = arguments(table)?;
      Target::TextObject(
        object(&argument.object)
          .ok_or_else(|| anyhow::anyhow!("Unknown text object"))?,
      )
    } else {
      Target::Reader(reader(name, table)?)
    }
  };
  anyhow::ensure!(
    target.allowed(context),
    "Action unavailable in {}",
    context.name()
  );
  Ok(Some(target))
}

fn arguments<T: serde::de::DeserializeOwned>(
  table: &toml::Table,
) -> crate::Result<T> {
  let mut table = table.clone();
  table.remove("action");
  Ok(Value::Table(table).try_into()?)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Integer {
  amount: i32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Number {
  amount: f32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MotionArg {
  motion: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectArg {
  object: String,
  around: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextObject {
  object: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkArg {
  name: char,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoMarkArg {
  name: char,
  exact: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShiftArg {
  dx: i8,
  dy: i8,
}

fn reader(
  name: &str,
  table: &toml::Table,
) -> crate::Result<vpdf_ui::keys::Action> {
  let spec = actions::lookup(name)
    .ok_or_else(|| anyhow::anyhow!("Unknown action {name}"))?;
  let argument = match spec.kind {
    Kind::Fixed(_) => {
      let _: NoArgs = arguments(table)?;
      Argument::None
    }
    Kind::Motion => {
      let argument: MotionArg = arguments(table)?;
      Argument::Motion(
        motion(&argument.motion)
          .ok_or_else(|| anyhow::anyhow!("Unknown motion"))?,
      )
    }
    Kind::Object => {
      let argument: ObjectArg = arguments(table)?;
      Argument::Object {
        kind: object(&argument.object)
          .ok_or_else(|| anyhow::anyhow!("Unknown text object"))?,
        around: argument.around,
      }
    }
    Kind::Scroll | Kind::Align => {
      Argument::Number(arguments::<Number>(table)?.amount)
    }
    Kind::Screen | Kind::Rows | Kind::Zoom => {
      let amount = arguments::<Integer>(table)?.amount;
      anyhow::ensure!(
        (-10_000..=10_000).contains(&amount),
        "Amount outside supported range"
      );
      Argument::Integer(amount)
    }
    Kind::Mark => Argument::Character(arguments::<MarkArg>(table)?.name),
    Kind::GoMark => {
      let argument: GoMarkArg = arguments(table)?;
      Argument::Mark { name: argument.name, exact: argument.exact }
    }
    Kind::RegionShift => {
      let argument: ShiftArg = arguments(table)?;
      Argument::Shift { dx: argument.dx, dy: argument.dy }
    }
    Kind::Window => {
      let argument = arguments::<WindowArg>(table)?;
      if let WindowArg::Resize { amount, .. } = argument {
        anyhow::ensure!(
          (-10_000..=10_000).contains(&amount),
          "Amount outside supported range"
        );
      }
      Argument::Window(argument.into())
    }
  };
  spec.resolve(argument)
}

fn motion(name: &str) -> Option<vpdf_reader::motion::Motion> {
  actions::MOTIONS.iter().find(|(key, _)| *key == name).map(|(_, value)| *value)
}

fn object(name: &str) -> Option<vpdf_reader::objects::Object> {
  actions::OBJECTS.iter().find(|(key, _)| *key == name).map(|(_, value)| *value)
}
