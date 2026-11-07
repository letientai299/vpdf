//! Selects release or development versions from Git tags.
use std::path::Path;
use std::process::Command;

#[path = "../../src/provenance.rs"]
mod provenance;

mod help;
mod identity;

fn git(args: &[&str]) -> Option<String> {
  let output = Command::new("git").args(args).output().ok()?;
  let text = String::from_utf8(output.stdout).ok()?.trim().to_owned();
  (output.status.success() && !text.is_empty()).then_some(text)
}

/// Tags at HEAD; ignores enclosing repositories for source archives.
fn tags() -> Option<String> {
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
  if let Some(files) = git(&["ls-files"]) {
    for path in files.lines() {
      println!("cargo::rerun-if-changed={path}");
    }
  }
  git(&["tag", "--points-at", "HEAD"])
}

/// Shares the release workflow's notes renderer.
fn notes(tag: &str) -> String {
  if git(&["rev-parse", "--is-shallow-repository"]).as_deref() == Some("true") {
    println!("cargo::warning=Release notes need full history; omitting them");
    return String::new();
  }
  let output = Command::new("python3")
    .args(["-B", "-m", "tools.package.notes", tag])
    .output()
    .expect("Release notes need Python 3.11; run mise install");
  assert!(
    output.status.success(),
    "Release notes failed: {}",
    String::from_utf8_lossy(&output.stderr)
  );
  let text = String::from_utf8(output.stdout).unwrap();
  text.strip_suffix('\n').unwrap_or(&text).to_owned()
}

fn main() {
  // Overlaps help facts with the Git queries below.
  let assets = std::thread::spawn(help::prepare);
  let tags = tags().unwrap_or_default();
  let version = identity::version(&tags);
  println!("cargo::rustc-env=VPDF_VERSION={version}");
  // Only stable tags publish notes for CI to verify.
  let notes = identity::release(&tags)
    .filter(|release| release.pre.is_empty() && release.build.is_empty())
    .map_or_else(String::new, |release| notes(&format!("v{release}")));
  let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
  std::fs::write(output.join("changelog.md"), notes).unwrap();
  for path in ["src", "crates", "tools", ".config", "Cargo.toml", "Cargo.lock"]
  {
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
  if let Err(panic) = assets.join() {
    std::panic::resume_unwind(panic);
  }
}
