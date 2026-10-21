use crate::Result;
use serde_json::Value;

#[derive(Debug, Eq, PartialEq)]
pub struct Expectation {
  field: String,
  value: String,
}

impl Expectation {
  pub const HELP: &str = "Assertions: expect FIELD VALUE\n\
Fields: document (PDF stem), page, pages, pane, pane_count, tab,\n\
hits, hits_min,\n\
selected_hit (one-based), selected_hit_min, query, query_generation,\n\
query_generation_min,\n\
document_generation, search_outcome (idle/running/success/cancelled/error),\n\
panel (title), hidden_released, settled, sharp, overview (true/false).\n\
moved MARK, selected_changed MARK compare named checkpoints.\n\
Assertions use the run deadline.";

  pub fn parse(text: &str) -> Result<Self> {
    let (field, value) = text
      .split_once(char::is_whitespace)
      .ok_or_else(|| anyhow::anyhow!("Expectation needs field and value"))?;
    let value = value.trim();
    match field {
      "document" | "query" | "panel" | "moved" | "selected_changed" => {
        anyhow::ensure!(!value.is_empty(), "Expectation value empty");
      }
      "search_outcome" => anyhow::ensure!(
        matches!(value, "idle" | "running" | "success" | "cancelled" | "error"),
        "Invalid search outcome"
      ),
      "page"
      | "pages"
      | "pane_count"
      | "pane"
      | "tab"
      | "hits"
      | "hits_min"
      | "selected_hit_min"
      | "selected_hit"
      | "query_generation"
      | "query_generation_min"
      | "document_generation" => {
        value.parse::<u64>()?;
      }
      "hidden_released" | "settled" | "sharp" | "overview" => anyhow::ensure!(
        matches!(value, "true" | "false"),
        "Invalid expectation boolean"
      ),
      _ => anyhow::bail!("Unknown expectation field: {field}"),
    }
    Ok(Self { field: field.into(), value: value.into() })
  }

  pub fn checkpoint(&self) -> Option<&str> {
    matches!(self.field.as_str(), "moved" | "selected_changed")
      .then_some(self.value.as_str())
  }

  pub fn matches(&self, row: &Value) -> bool {
    self.matches_from(row, None)
  }

  pub fn matches_from(&self, row: &Value, baseline: Option<&Value>) -> bool {
    let field = self.field.as_str();
    let value = self.value.as_str();
    match field {
      "moved" => baseline.is_some_and(|before| {
        same_pane(row, before)
          && ["page", "scroll"].iter().all(|key| {
            row[key].as_f64().is_some_and(f64::is_finite)
              && before[key].as_f64().is_some_and(f64::is_finite)
          })
          && (row["page"] != before["page"]
            || row["scroll"] != before["scroll"])
      }),
      "selected_changed" => baseline.is_some_and(|before| {
        same_pane(row, before)
          && row["query_generation"].as_u64().is_some()
          && row["query_generation"] == before["query_generation"]
          && row["selected_hit"].as_u64().is_some_and(|hit| hit > 0)
          && before["selected_hit"].as_u64().is_some_and(|hit| hit > 0)
          && row["selected_hit"] != before["selected_hit"]
      }),
      "document" => row[field]
        .as_str()
        .and_then(|p| std::path::Path::new(p).file_stem())
        .is_some_and(|name| name == value),
      "query" | "panel" | "search_outcome" => {
        row[field].as_str() == Some(value)
      }
      "hidden_released" => {
        row["resources"].as_array().is_some_and(|resources| {
          let hidden: Vec<_> =
            resources.iter().filter(|r| r["visible"] == false).collect();
          (!hidden.is_empty()
            && hidden.iter().all(|r| {
              r["text_bytes"] == 0
                && r["tile_bytes"] == 0
                && r["search_workers"] == 0
                && r["active_workers"] == 0
            }))
            == (value == "true")
        })
      }
      "overview" => row["overview"].as_bool() == Some(value == "true"),
      "settled" => {
        row["animating"].is_boolean()
          && row["busy"].is_boolean()
          && ((row["animating"] == false && row["busy"] == false)
            == (value == "true"))
      }
      "sharp" => {
        row["complete"].is_boolean()
          && row["busy"].is_boolean()
          && row["missing"].is_number()
          && row["stand_ins"].is_number()
          && ((row["complete"] == true
            && row["busy"] == false
            && row["missing"] == 0
            && row["stand_ins"] == 0)
            == (value == "true"))
      }
      _ => {
        let key = field.strip_suffix("_min").unwrap_or(field);
        row[key].as_u64().is_some_and(|actual| {
          let Ok(expected) = value.parse::<u64>() else { return false };
          if key == field { actual == expected } else { actual >= expected }
        })
      }
    }
  }
}

fn same_pane(row: &Value, before: &Value) -> bool {
  row["document"].as_str().is_some()
    && row["document"] == before["document"]
    && row["pane"].as_u64().is_some()
    && row["pane"] == before["pane"]
    && row["document_generation"].as_u64().is_some()
    && row["document_generation"] == before["document_generation"]
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn movement_requires_checkpoint_change() -> Result<()> {
    let before = json!({"pane":1,"document":"sdm.pdf",
      "document_generation":0,"page":1,"scroll":0.0,
      "selected_hit":1,"query_generation":8});
    for field in ["moved keys", "selected_changed results"] {
      let assertion = Expectation::parse(field)?;
      assert!(!assertion.matches_from(&before, Some(&before)));
      assert!(!assertion.matches_from(&before, None));
    }
    let moved = Expectation::parse("moved keys")?;
    let selected = Expectation::parse("selected_changed results")?;
    let mut after = before.clone();
    after["scroll"] = 10.0.into();
    assert!(moved.matches_from(&after, Some(&before)));
    after["selected_hit"] = 2.into();
    assert!(selected.matches_from(&after, Some(&before)));
    after["query_generation"] = 9.into();
    assert!(!selected.matches_from(&after, Some(&before)));
    after["pane"] = 2.into();
    assert!(!moved.matches_from(&after, Some(&before)));
    Ok(())
  }

  #[test]
  fn idle_is_not_success() -> Result<()> {
    for (spec, row) in [
      ("page 5", json!({"page":1,"busy":false,"complete":true})),
      ("hits_min 1", json!({"hits":0,"busy":false,"complete":true})),
      (
        "search_outcome success",
        json!({"search_outcome":"error","busy":false}),
      ),
      ("sharp true", json!({"complete":true,"busy":false})),
      ("hidden_released true", json!({"resources":[]})),
    ] {
      assert!(!Expectation::parse(spec)?.matches(&row), "{spec}");
    }
    assert!(Expectation::parse("hits_min 1")?.matches(&json!({"hits":2})));
    assert!(
      Expectation::parse("document sdm")?
        .matches(&json!({"document":"/tmp/sdm.pdf"}))
    );
    Ok(())
  }
}
