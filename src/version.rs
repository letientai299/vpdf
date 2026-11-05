use std::fmt::Write as _;
use std::io::{self, Write};

pub const ABOUT: &str =
  concat!(env!("CARGO_PKG_DESCRIPTION"), "\n", env!("CARGO_PKG_REPOSITORY"));
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
pub const VERSION: &str = env!("VPDF_VERSION");
pub const REVISION: &str = env!("VPDF_SOURCE_REVISION");
pub const DIRTY: &str = env!("VPDF_SOURCE_DIRTY");
pub const PATCH: &str = env!("VPDF_SOURCE_PATCH");

pub const BUILD: &str = if cfg!(feature = "bench-events") {
  concat!(env!("VPDF_VERSION"), " [bench-events]")
} else {
  env!("VPDF_VERSION")
};

pub const CHANGELOG: &str =
  include_str!(concat!(env!("OUT_DIR"), "/changelog.md"));

/// Builds other than clean stable releases name their source.
fn sourced() -> bool {
  DIRTY != "false"
    || VERSION == "dev"
    || VERSION.contains('-')
    || cfg!(feature = "bench-events")
}

pub fn text(terminal: bool) -> String {
  crate::manual::identity();
  let mut text = name(terminal);
  if sourced() {
    write!(text, "\nbuild-source {REVISION} {DIRTY} {PATCH}").unwrap();
  }
  if !CHANGELOG.is_empty() {
    text.push_str("\n\n");
    text.push_str(CHANGELOG);
  }
  write!(text, "\n\n{REPOSITORY}").unwrap();
  text
}

pub fn panel_text(terminal: bool) -> String {
  text(terminal)
}

fn name(terminal: bool) -> String {
  let name = if terminal { "vpdf" } else { "vpdf-gui" };
  format!("{name} {BUILD}")
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
  fn notes_match_frontends() {
    for terminal in [true, false] {
      let text = super::text(terminal);
      assert_eq!(text.lines().next(), Some(super::name(terminal).as_str()));
      if super::sourced() {
        assert_eq!(
          text.lines().nth(1).unwrap(),
          format!(
            "build-source {} {} {}",
            super::REVISION,
            super::DIRTY,
            super::PATCH
          )
        );
      }
      assert!(!text.contains("\n\n\n"));
      assert!(text.contains(super::CHANGELOG));
      assert!(text.ends_with(super::REPOSITORY));
      assert!(!text.contains("Source revision:"));
      assert_eq!(text, super::panel_text(terminal));
    }
  }
}

#[cfg(test)]
#[path = "../tools/build/identity.rs"]
mod identity;
