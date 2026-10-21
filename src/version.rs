use std::io::{self, Write};

pub const ABOUT: &str =
  concat!(env!("CARGO_PKG_DESCRIPTION"), "\n", env!("CARGO_PKG_REPOSITORY"));

pub const BUILD: &str = if cfg!(feature = "bench-events") {
  concat!(env!("VPDF_VERSION"), " [bench-events]")
} else {
  env!("VPDF_VERSION")
};

pub fn text(terminal: bool) -> String {
  let name = if terminal { "vpdf" } else { "vpdf-gui" };
  let revision = env!("VPDF_SOURCE_REVISION");
  let dirty = env!("VPDF_SOURCE_DIRTY");
  let patch = env!("VPDF_SOURCE_PATCH");
  format!("{ABOUT}\nbuild-source {revision} {dirty} {patch}\n{name} {BUILD}")
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
