use std::io::{self, Write};

pub const ABOUT: &str =
  concat!(env!("CARGO_PKG_DESCRIPTION"), "\n", env!("CARGO_PKG_REPOSITORY"));

pub const BUILD: &str = if cfg!(feature = "bench-events") {
  concat!(env!("VPDF_VERSION"), " [bench-events]")
} else {
  env!("VPDF_VERSION")
};

pub fn text(terminal: bool) -> String {
  let [source, help] = build();
  format!("{ABOUT}\n{source}\n{help}\n{}", name(terminal))
}

/// The panel's text: the name and summary, then a `# Build` section.
pub fn panel_text(terminal: bool) -> String {
  let [source, help] = build();
  let build = vpdf_ui::panel::BUILD;
  format!("{}\n{ABOUT}\n# {build}\n{source}\n{help}", name(terminal))
}

fn name(terminal: bool) -> String {
  let name = if terminal { "vpdf" } else { "vpdf-gui" };
  format!("{name} {BUILD}")
}

/// Source and help asset identities.
fn build() -> [String; 2] {
  let revision = env!("VPDF_SOURCE_REVISION");
  let dirty = env!("VPDF_SOURCE_DIRTY");
  let patch = env!("VPDF_SOURCE_PATCH");
  let help = crate::manual::identity();
  [
    format!("build-source {revision} {dirty} {patch}"),
    format!("help-assets {help}"),
  ]
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
