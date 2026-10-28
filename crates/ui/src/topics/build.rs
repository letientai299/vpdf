use super::{Destination, Registry, authored};
use crate::bindings::Bindings;
use serde_json::{Value, json};

pub fn facts() -> crate::Result<Value> {
  let defaults = Bindings::default();
  let bindings = defaults.snapshot();
  let registry = Registry::new(authored(), bindings.clone())?;
  let topics: Vec<_> = registry
    .topics()
    .map(|topic| {
      json!({
        "id": topic.id.as_str(),
        "title": topic.title,
        "aliases": topic.aliases,
        "kind": match topic.destination {
          Destination::Authored(_) => "authored",
          Destination::Action(_) | Destination::Key(_) => "action",
          Destination::Command(_) => "command",
          Destination::Catalog(_) => "catalog",
          Destination::Setting(_) => "setting",
          Destination::Configuration => "configuration",
        },
      })
    })
    .collect();
  let defaults: Vec<_> = bindings
    .defaults
    .iter()
    .filter(|row| !row.is_prefix())
    .map(|row| {
      json!({
        "action": row.target.topic(),
        "mode": row.context.name(),
        "keys": row.sequence.to_string(),
        "purpose": row.purpose,
        "preferred": defaults.shortcut(row.context, &row.target)
          == row.sequence.to_string(),
      })
    })
    .collect();
  Ok(json!({"schema": 1, "topics": topics, "defaults": defaults}))
}
