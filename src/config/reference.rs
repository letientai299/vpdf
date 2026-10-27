use super::Settings;
use std::collections::BTreeMap;
use vpdf_ui::reference::{Configuration, Setting, Source};

#[derive(Clone, Debug, Default)]
pub(super) struct Resolved {
  pub generation: u64,
  pub sources: BTreeMap<String, Source>,
  pub warnings: Vec<String>,
}

impl Resolved {
  pub fn inherit(&mut self, previous: &Self, section: &str) {
    let prefix = format!("{section}.");
    self.sources.extend(
      previous
        .sources
        .iter()
        .filter(|(name, _)| name.starts_with(&prefix))
        .map(|(name, source)| (name.clone(), source.clone())),
    );
  }
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
        "appearance.opacity",
        serde_json::to_value(
          defaults.opacity().values().collect::<BTreeMap<_, _>>(),
        ),
        serde_json::to_value(
          self.opacity().values().collect::<BTreeMap<_, _>>(),
        ),
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
      let effective = effective.unwrap_or_default();
      let Some(table) = effective.as_object() else { continue };
      for (key, value) in table
        .iter()
        .filter(|(key, _)| section != "appearance" || key.as_str() != "opacity")
      {
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
    Configuration {
      generation: self.reference.generation,
      palette: self.colors(vpdf_model::theme::Colors::Terminal).palette(),
      settings,
      warnings: self.reference.warnings.clone(),
    }
  }
}
