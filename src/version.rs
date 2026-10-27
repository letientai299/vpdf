use std::fmt::Write as _;
use std::io::{self, Write};

pub const ABOUT: &str =
  concat!(env!("CARGO_PKG_DESCRIPTION"), "\n", env!("CARGO_PKG_REPOSITORY"));
pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const REVISION: &str = env!("VPDF_SOURCE_REVISION");
const DIRTY: &str = env!("VPDF_SOURCE_DIRTY");
const PATCH: &str = env!("VPDF_SOURCE_PATCH");

pub const BUILD: &str = if cfg!(feature = "bench-events") {
  concat!(env!("VPDF_VERSION"), " [bench-events]")
} else {
  env!("VPDF_VERSION")
};

pub fn text(terminal: bool) -> String {
  format!("{ABOUT}\n\n{}\n\n{}", build_details(), name(terminal))
}

/// The panel's text: the name and summary, then a `# Build` section.
pub fn panel_text(terminal: bool) -> String {
  let build = vpdf_ui::panel::BUILD;
  format!("{}\n{ABOUT}\n# {build}\n{}", name(terminal), build_details())
}

fn name(terminal: bool) -> String {
  let name = if terminal { "vpdf" } else { "vpdf-gui" };
  format!("{name} {BUILD}")
}

pub fn build_details() -> String {
  details(REVISION, DIRTY, PATCH, &crate::manual::identity())
}

fn details(revision: &str, dirty: &str, patch: &str, help: &str) -> String {
  let source = match dirty {
    "true" => "Includes uncommitted changes; links show the base commit.",
    "false" => "Built from an unmodified source tree.",
    _ => "Source tree status is unavailable.",
  };
  let mut text = format!(
    "Source revision: {revision}\n\
     Git commit used to build this executable.\n"
  );
  if revision != "unknown" {
    writeln!(text, "{REPOSITORY}/commit/{revision}").unwrap();
  }
  write!(
    text,
    "\nLocal changes: {dirty}\n{source}\n\n\
     Source patch: {patch}\n\
     Git blob hash of the local source changes.\n\
     Local fingerprint; cannot open as a GitHub commit.\n\n\
     Help assets: {help}\n\
     SHA-256 of help inputs and generated reference metadata.\n\
     Identifies bundled help; this is not a Git commit."
  )
  .unwrap();
  if revision != "unknown" {
    write!(text, "\n{REPOSITORY}/blob/{revision}/scripts/help_assets.py")
      .unwrap();
  }
  text
}

pub fn parse<T: clap::Parser>(terminal: bool) -> T {
  match T::try_parse() {
    Ok(args) => args,
    Err(error) if error.kind() == clap::error::ErrorKind::DisplayVersion => {
      let code = match writeln!(io::stdout().lock(), "{}", text(terminal)) {
        Ok(()) => 0,
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => 0,
        Err(error) => {
          eprintln!("Version output failed: {error}");
          1
        }
      };
      std::process::exit(code);
    }
    Err(error) => error.exit(),
  }
}

#[cfg(test)]
mod tests {
  #[test]
  fn diagnostics_match_frontends() {
    let details = super::build_details();
    for terminal in [true, false] {
      assert!(super::text(terminal).contains(&details));
      assert!(super::panel_text(terminal).contains(&details));
    }
  }

  #[test]
  fn source_links_respect_provenance() {
    for (dirty, explanation) in [
      ("true", "Includes uncommitted changes"),
      ("false", "unmodified source tree"),
      ("unknown", "status is unavailable"),
    ] {
      let text = super::details("abc123", dirty, "patch", "help");
      assert!(text.contains(explanation));
      assert!(text.contains("/commit/abc123"));
      assert!(text.contains("/blob/abc123/scripts/help_assets.py"));
    }
    let text = super::details("unknown", "unknown", "unknown", "help");
    assert!(!text.contains("/commit/"));
    assert!(!text.contains("/blob/"));
  }
}
