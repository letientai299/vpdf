//! Configuration action entries.

use crate::Result;
use std::collections::BTreeMap;
use std::fmt::Write;
use vpdf_pdf::manual::{Chapter, Line, Section};
use vpdf_ui::bindings::catalog::{Family, Field, Group, families};
use vpdf_ui::bindings::{Binding, Bindings, Context};

/// The example key; users replace it.
const KEY: &str = "<F2>";

/// Contexts where the parser accepts a binding target.
pub(super) fn contexts(table: &str) -> Vec<Context> {
  Context::ALL
    .iter()
    .copied()
    .filter(|&context| crate::config::binding_target(table, context).is_ok())
    .collect()
}

fn names(contexts: &[Context]) -> String {
  contexts.iter().map(|context| context.name()).collect::<Vec<_>>().join(", ")
}

fn inline_fields(fields: &[Field]) -> String {
  fields
    .iter()
    .map(|field| format!("`{}`: {}", field.name, field.value.constraint()))
    .collect::<Vec<_>>()
    .join("; ")
}

/// A `Fields:` heading and one item per field.
fn fields(fields: &[Field]) -> Vec<Line> {
  if fields.is_empty() {
    return Vec::new();
  }
  let mut lines = vec![Line::Text("Fields:".into())];
  lines.extend(fields.iter().map(|field| {
    Line::Item(format!("`{}`: {}.", field.name, field.value.constraint()))
  }));
  lines
}

pub(super) fn chapters(
  defaults: &Bindings,
  guides: &BTreeMap<String, String>,
  teaching: impl Fn(&str) -> &'static str,
  shortcut: impl Fn(&Binding) -> String,
) -> Result<Vec<Chapter>> {
  let mut families = families()?;
  families.sort_by_key(|family| family.group);
  let snapshot = defaults.snapshot();
  let mut chapters = Vec::new();
  for family in &families {
    let contexts = contexts(&family.table());
    anyhow::ensure!(!contexts.is_empty(), "Unavailable catalog example");
    let mut lines = vec![
      Line::Binding {
        keys: family.name.into(),
        description: format!(
          "{} Contexts: {}. {}",
          family.purpose,
          names(&contexts),
          keys(family, &snapshot.defaults, &shortcut)
        ),
      },
      Line::Code(format!("[keybindings.{}]", contexts[0].name())),
      Line::Code(format!("\"{}\" = {}", example_key(family), family.table())),
    ];
    lines.extend(fields(family.fields));
    if !family.variants.is_empty() {
      lines.push(Line::Text("Operations:".into()));
    }
    for variant in family.variants {
      let mut item = format!("`{}`: {}", variant.name, variant.purpose);
      let own = contexts_of(family, variant);
      if own != contexts {
        let _ = write!(item, " Contexts: {}.", names(&own));
      }
      if !variant.fields.is_empty() {
        let _ = write!(item, " Fields: {}.", inline_fields(variant.fields));
      }
      lines.push(Line::Item(item));
    }
    let guide = if family.group == Group::Target {
      "configuration"
    } else {
      teaching(family.name)
    };
    chapters.push(Chapter {
      id: family.topic(),
      title: family.name.into(),
      lines,
      related: guide.into(),
      related_title: guides[guide].clone(),
      anchors: BTreeMap::new(),
      section: Some(Section::Actions),
      listed: true,
      aliases: Vec::new(),
      links: Vec::new(),
    });
  }
  Ok(chapters)
}

/// Names a few default keys; the key section lists all.
fn keys(
  family: &Family,
  defaults: &[Binding],
  shortcut: impl Fn(&Binding) -> String,
) -> String {
  const SHOWN: usize = 3;
  let mut keys: Vec<String> = Vec::new();
  let topic = family.topic();
  let mut rows: Vec<_> = defaults
    .iter()
    .filter(|row| Family::topic_of(&row.target) == topic)
    .collect();
  rows.sort_by_key(|row| Context::ALL.iter().position(|&c| c == row.context));
  for row in rows {
    let key = shortcut(row);
    if !keys.contains(&key) {
      keys.push(key);
    }
  }
  let mut shown: Vec<_> =
    keys.iter().take(SHOWN).map(|key| span(key)).collect();
  if keys.len() > SHOWN {
    shown.push(format!("and {} more", keys.len() - SHOWN));
  }
  if shown.is_empty() {
    "Default keys: none.".into()
  } else {
    format!("Default keys: {}.", shown.join(", "))
  }
}

/// A code span; doubled fences admit backticks.
fn span(key: &str) -> String {
  if key.contains('`') { format!("`` {key} ``") } else { format!("`{key}`") }
}

fn contexts_of(
  family: &Family,
  variant: &vpdf_ui::bindings::catalog::Variant,
) -> Vec<Context> {
  contexts(&family.variant_table(variant))
}

fn example_key(family: &Family) -> &'static str {
  match family.name {
    "unbind" => "gt",
    _ => KEY,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_ui::bindings::catalog::Value;

  fn target(table: &str) -> toml::Table {
    let mut value: toml::Table =
      toml::from_str(&format!("target = {table}")).unwrap();
    let toml::Value::Table(target) = value.remove("target").unwrap() else {
      panic!("{table}: not a table");
    };
    target
  }

  /// Inline TOML that parses back to `target`.
  fn inline(target: &toml::Table) -> String {
    let fields: Vec<_> =
      target.iter().map(|(key, value)| format!("{key} = {value}")).collect();
    let table = format!("{{ {} }}", fields.join(", "));
    // Debug text compares NaN samples equal.
    assert_eq!(format!("{:?}", self::target(&table)), format!("{target:?}"));
    table
  }

  fn without(table: &str, field: &str) -> String {
    let mut target = target(table);
    assert!(target.remove(field).is_some(), "{table}: no {field}");
    inline(&target)
  }

  fn with(table: &str, field: &str, raw: &str) -> String {
    let fields: toml::Table =
      toml::from_str(&format!("{field} = {raw}")).unwrap();
    let mut target = target(table);
    target.extend(fields);
    inline(&target)
  }

  fn check(family: &Family, table: &str, fields: &[Field]) {
    let accepted = contexts(table);
    assert!(!accepted.is_empty(), "{table}");
    for &context in Context::ALL {
      let parsed = crate::config::binding_target(table, context);
      if accepted.contains(&context) {
        if let Some(target) = parsed.unwrap() {
          assert_eq!(Family::topic_of(&target), family.topic(), "{table}");
        } else {
          assert_eq!(family.name, "unbind");
        }
      } else {
        let error = parsed.unwrap_err().to_string();
        assert!(error.contains("unavailable"), "{table} {context:?}: {error}");
      }
    }
    let context = accepted[0];
    for field in fields {
      let missing = without(table, field.name);
      assert!(
        crate::config::binding_target(&missing, context).is_err(),
        "{missing}"
      );
      if let Some(raw) = field.value.invalid() {
        let invalid = with(table, field.name, &raw);
        assert!(
          crate::config::binding_target(&invalid, context).is_err(),
          "{invalid}"
        );
      }
      if field.value == Value::Bool {
        let invalid = with(table, field.name, "\"yes\"");
        assert!(crate::config::binding_target(&invalid, context).is_err());
      }
    }
    let extra = with(table, "unknown_field", "1");
    assert!(crate::config::binding_target(&extra, context).is_err(), "{extra}");
  }

  #[test]
  fn examples_agree_with_parser() {
    let families = families().unwrap();
    for family in &families {
      check(family, &family.table(), family.fields);
      for variant in family.variants {
        check(family, &family.variant_table(variant), variant.fields);
      }
    }
    let mut topics: Vec<_> = families.iter().map(Family::topic).collect();
    topics.sort();
    topics.dedup();
    assert_eq!(topics.len(), families.len());
    for row in &Bindings::default().snapshot().defaults {
      let topic = Family::topic_of(&row.target);
      assert!(topics.contains(&topic), "{topic}");
    }
    for table in [
      "{ action = \"unknown\" }",
      "{ action = \"prompt.unknown\" }",
      "{ action = \"hint.unknown\" }",
    ] {
      assert!(crate::config::binding_target(table, Context::Normal).is_err());
    }
  }

  #[test]
  fn entries_cover_unbound_families() -> Result<()> {
    let chapters = super::super::reference::chapters()?;
    // Name, entry text, code block, and field items.
    let entry = |id: &str| {
      let chapter = chapters.iter().find(|row| row.id == id).unwrap();
      assert_eq!(chapter.section, Some(Section::Actions));
      let Line::Binding { keys, description } = &chapter.lines[0] else {
        panic!("{id} starts without an entry row");
      };
      let mut code = Vec::new();
      let mut items = Vec::new();
      for line in &chapter.lines[1..] {
        match line {
          Line::Code(text) => code.push(text.clone()),
          Line::Item(text) => items.push(text.clone()),
          Line::Binding { .. } => break,
          Line::Text(_) => {}
        }
      }
      (keys.clone(), description.clone(), code, items)
    };
    let defaults = Bindings::default().snapshot().defaults;
    let mut unbound = 0;
    for family in families()? {
      let (keys, text, code, items) = entry(&family.topic());
      assert_eq!(keys, family.name);
      assert!(code[0].starts_with("[keybindings."), "{code:?}");
      assert!(code[1].ends_with(&family.table()), "{code:?}");
      assert_eq!(items.len(), family.fields.len() + family.variants.len());
      for (item, field) in items.iter().zip(family.fields) {
        assert!(item.starts_with(&format!("`{}`", field.name)), "{item}");
      }
      let bound = defaults
        .iter()
        .any(|row| Family::topic_of(&row.target) == family.topic());
      let none = text.ends_with("Default keys: none.");
      assert_eq!(none, !bound, "{}", family.name);
      unbound += usize::from(none);
    }
    assert!(unbound > 0);
    let (_, scroll, ..) = entry("action.scroll");
    assert!(scroll.contains("<C-d>"), "{scroll}");
    let (_, window, ..) = entry("action.window");
    assert!(window.ends_with("more."), "{window}");
    assert!(!chapters.iter().any(|chapter| chapter.id == "actions"));
    Ok(())
  }
}
