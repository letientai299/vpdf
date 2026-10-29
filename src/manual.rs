use sha2::{Digest, Sha256};

mod catalog;
mod reference;
mod runtime;
pub use runtime::{Location, Ready, Runtime};

pub(crate) fn palette() -> vpdf_model::theme::Palette {
  vpdf_model::theme::Palette::light()
}

pub const ID: &str = "builtin:help";

pub const PDF: &[u8] =
  include_bytes!(concat!(env!("VPDF_HELP_DIR"), "/base.pdf"));
pub const TOPICS: &str =
  include_str!(concat!(env!("VPDF_HELP_DIR"), "/topics.json"));
pub const MANIFEST: &str =
  include_str!(concat!(env!("VPDF_HELP_DIR"), "/manifest.json"));
pub const NOTICES: &str =
  include_str!(concat!(env!("VPDF_HELP_DIR"), "/third-party-notices.txt"));

pub fn identity() -> String {
  static IDENTITY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
  IDENTITY.get_or_init(verify_identity).clone()
}

fn verify_identity() -> String {
  let manifest: serde_json::Value =
    serde_json::from_str(MANIFEST).expect("Verified help manifest");
  for (name, bytes) in [
    ("base.pdf", PDF),
    ("topics.json", TOPICS.as_bytes()),
    ("third-party-notices.txt", NOTICES.as_bytes()),
  ] {
    assert_eq!(
      manifest["outputs"][name].as_str(),
      Some(vpdf_pdf::content_cache::hex(&Sha256::digest(bytes)).as_str()),
      "Embedded help hash mismatch"
    );
  }
  manifest["fingerprint"].as_str().expect("Help fingerprint").to_owned()
}

#[cfg(test)]
mod tests;

pub(crate) fn completions(
  settings: &crate::config::Settings,
  invocation: &vpdf_ui::topics::Invocation,
  prefix: &str,
) -> crate::Result<Vec<vpdf_ui::topics::Candidate>> {
  let reference = settings.help_reference(invocation.clone());
  Ok(registry(&reference)?.complete(prefix, invocation.context()))
}

/// The topic registry for a help snapshot, including setting names.
pub(crate) fn registry(
  reference: &vpdf_ui::topics::Reference,
) -> crate::Result<vpdf_ui::topics::Registry> {
  let settings = &reference.configuration.settings;
  let titles: Vec<_> = settings
    .iter()
    .map(|setting| crate::config::describe(&setting.name))
    .collect::<crate::Result<_>>()?;
  vpdf_ui::topics::Registry::new(
    vpdf_ui::topics::authored(),
    reference.bindings.clone(),
  )?
  .with_settings(
    settings
      .iter()
      .zip(&titles)
      .map(|(setting, field)| (setting.name.as_str(), field.summary)),
  )
}
