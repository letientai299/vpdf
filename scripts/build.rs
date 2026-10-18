//! Names the build for `--version`: its release tag, else its commit
//! and commit date. Also names the locked crates that shape search
//! records, so a bump retires records cached on disk.
use std::path::Path;
use std::process::Command;

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

/// `name version source` of each locked crate in `names`; empty unless
/// the lock names them all.
fn locked(names: &[&str]) -> String {
  let lock = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.lock");
  let Ok(text) = std::fs::read_to_string(&lock) else {
    return String::new();
  };
  println!("cargo::rerun-if-changed={}", lock.display());
  let mut found = Vec::new();
  for package in text.split("[[package]]") {
    let field = |key: &str| {
      package.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.strip_prefix(" = ")?;
        Some(value.trim_matches('"'))
      })
    };
    if field("name").is_some_and(|name| names.contains(&name)) {
      let [name, version, source] =
        ["name", "version", "source"].map(|key| field(key).unwrap_or_default());
      found.push(format!("{name} {version} {source}"));
    }
  }
  if found.len() < names.len() {
    return String::new();
  }
  found.join("; ")
}

/// A digest of the sources that extract and index page text.
fn code(paths: &[&str]) -> u64 {
  use std::hash::{DefaultHasher, Hash, Hasher};
  let mut hasher = DefaultHasher::new();
  for path in paths {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    println!("cargo::rerun-if-changed={}", path.display());
    std::fs::read(&path).unwrap_or_default().hash(&mut hasher);
  }
  hasher.finish()
}

fn main() {
  let version =
    version().unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
  println!("cargo::rustc-env=VPDF_VERSION={version}");
  let index =
    locked(&["mupdf-sys", "unicode-casefold", "unicode-normalization"]);
  println!("cargo::rustc-env=VPDF_INDEX_CRATES={index}");
  let code = code(&["src/document.rs", "src/search.rs", "src/search/codec.rs"]);
  println!("cargo::rustc-env=VPDF_INDEX_CODE={code:016x}");
}
