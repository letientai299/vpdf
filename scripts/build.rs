//! Names the build for `--version`: its release tag, else its commit
//! and commit date.
use std::path::Path;
use std::process::Command;

#[path = "../src/provenance.rs"]
mod provenance;

mod help_build;

fn git(args: &[&str]) -> Option<String> {
  let output = Command::new("git").args(args).output().ok()?;
  let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
  (output.status.success() && !text.is_empty()).then_some(text)
}

/// The commit naming this source tree, unless git found an enclosing
/// repository, as for an unpacked crate inside another checkout.
fn version() -> Option<String> {
  let top = git(&["rev-parse", "--show-toplevel"])?;
  let own = std::fs::canonicalize(top).ok()?
    == std::fs::canonicalize(env!("CARGO_MANIFEST_DIR")).ok()?;
  if !own {
    return None;
  }
  // `logs/HEAD` grows on every commit and checkout, even when refs are
  // packed; `refs/heads` stands in without it, but would also see other
  // worktrees' commits. A missing path would rerun this script on every
  // build.
  let watch = |name| {
    let path = git(&["rev-parse", "--git-path", name])?;
    Path::new(&path)
      .exists()
      .then(|| println!("cargo::rerun-if-changed={path}"))
  };
  for name in ["HEAD", "packed-refs", "refs/tags", "reftable"] {
    watch(name);
  }
  if watch("logs/HEAD").is_none() {
    watch("refs/heads");
  }
  let tag = git(&["describe", "--tags", "--exact-match", "--match", "v[0-9]*"]);
  if let Some(tag) = tag {
    return Some(tag.trim_start_matches('v').to_owned());
  }
  let revision = git(&["rev-parse", "--short", "HEAD"])?;
  let date = git(&["log", "-1", "--format=%cs"])?;
  Some(format!("{revision} ({date})"))
}

fn main() {
  help_build::prepare();
  let version =
    version().unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
  println!("cargo::rustc-env=VPDF_VERSION={version}");
  for path in [
    "src",
    "crates",
    "scripts",
    "benches",
    ".config",
    "Cargo.toml",
    "Cargo.lock",
  ] {
    println!("cargo::rerun-if-changed={path}");
  }
  let root = Path::new(env!("CARGO_MANIFEST_DIR"));
  let source = git(&["rev-parse", "--show-toplevel"])
    .filter(|top| std::fs::canonicalize(top).ok() == root.canonicalize().ok())
    .and_then(|_| provenance::Source::read(root));
  for (key, value) in [
    ("REVISION", source.as_ref().map(|s| s.revision.as_str())),
    ("PATCH", source.as_ref().map(|s| s.patch.as_str())),
    ("DIRTY", source.as_ref().map(|s| if s.dirty { "true" } else { "false" })),
  ] {
    println!(
      "cargo::rustc-env=VPDF_SOURCE_{key}={}",
      value.unwrap_or("unknown")
    );
  }
}
