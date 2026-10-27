use super::Settings;
use std::collections::BTreeMap;
use vpdf_ui::reference::{Configuration, Setting, Source};

#[derive(Clone, Debug, Default)]
pub(super) struct Resolved {
  pub generation: u64,
  pub sources: BTreeMap<String, Source>,
  pub warnings: Vec<String>,
}

impl Settings {
  pub fn help_reference(
    &self,
    invocation: vpdf_ui::topics::Invocation,
  ) -> vpdf_ui::topics::Reference {
    let mut bindings = vpdf_ui::bindings::Bindings::default().snapshot();
    if let Some(kind) = invocation.pane().and_then(|pane| pane.panel) {
      bindings = bindings.in_panel(kind);
    }
    vpdf_ui::topics::Reference {
      configuration: self.reference(),
      bindings,
      invocation,
    }
  }

  pub fn reference(&self) -> Configuration {
    let defaults = Self::default();
    let sections = [
      (
        "appearance",
        serde_json::to_value(&defaults.appearance),
        serde_json::to_value(&self.appearance),
      ),
      (
        "reading",
        serde_json::to_value(&defaults.reading),
        serde_json::to_value(&self.reading),
      ),
      (
        "ocr",
        serde_json::to_value(&defaults.ocr),
        serde_json::to_value(&self.ocr),
      ),
    ];
    let mut settings = Vec::new();
    for (section, defaults, effective) in sections {
      let defaults = defaults.unwrap_or_default();
      if let Some(table) = effective.unwrap_or_default().as_object() {
        for (key, value) in table {
          let name = format!("{section}.{key}");
          settings.push(Setting {
            source: self
              .reference
              .sources
              .get(&name)
              .cloned()
              .unwrap_or_default(),
            name,
            default: defaults[key].clone(),
            effective: value.clone(),
          });
        }
      }
    }
    Configuration {
      generation: self.reference.generation,
      palette: self.colors(vpdf_model::theme::Colors::Terminal).palette(),
      settings,
      warnings: self.reference.warnings.clone(),
    }
  }
}
