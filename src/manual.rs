use sha2::{Digest, Sha256};

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
  include_str!(concat!(env!("VPDF_HELP_DIR"), "/font-notices.txt"));

pub(crate) fn identity() -> String {
  static IDENTITY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
  IDENTITY.get_or_init(verify_identity).clone()
}

fn verify_identity() -> String {
  let manifest: serde_json::Value =
    serde_json::from_str(MANIFEST).expect("Verified help manifest");
  for (name, bytes) in [
    ("base.pdf", PDF),
    ("topics.json", TOPICS.as_bytes()),
    ("font-notices.txt", NOTICES.as_bytes()),
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
  let registry = vpdf_ui::topics::Registry::new(
    vpdf_ui::topics::authored(),
    reference.bindings,
  )?;
  Ok(registry.complete(prefix, invocation.context()))
}
