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

pub fn text(terminal: bool) -> String {
  crate::manual::identity();
  format!("{}\n\n{CHANGELOG}\n\n{REPOSITORY}", name(terminal))
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
      assert!(text.starts_with(&super::name(terminal)));
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
