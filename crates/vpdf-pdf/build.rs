use std::path::Path;

/// `name version source` of each locked crate in `names`; empty unless
/// the lock names them all.
fn locked(names: &[&str]) -> String {
  let lock =
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join("Cargo.lock");
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
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(path);
    println!("cargo::rerun-if-changed={}", path.display());
    std::fs::read(&path)
      .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
      .hash(&mut hasher);
  }
  hasher.finish()
}

fn main() {
  let index = locked(&[
    "mupdf-sys",
    "unicode-casefold",
    "unicode-normalization",
    "unicode-script",
  ]);
  println!("cargo::rustc-env=VPDF_INDEX_CRATES={index}");
  let figures = code(&[
    "crates/vpdf-pdf/src/figures.rs",
    "crates/vpdf-pdf/src/jobs/figures.rs",
    "crates/vpdf-pdf/src/content_cache/record.rs",
    "crates/vpdf-model/src/caption.rs",
  ]);
  println!("cargo::rustc-env=VPDF_FIGURE_CODE={figures:016x}");
  let recent = code(&[
    "crates/vpdf-pdf/src/jobs/recent.rs",
    "crates/vpdf-pdf/src/raster.rs",
    "crates/vpdf-model/src/image.rs",
  ]);
  println!("cargo::rustc-env=VPDF_RECENT_CODE={recent:016x}");
  let outline = code(&[
    "crates/vpdf-pdf/src/jobs/outline.rs",
    "crates/vpdf-pdf/src/content_cache/record.rs",
  ]);
  println!("cargo::rustc-env=VPDF_OUTLINE_CODE={outline:016x}");
  let code = code(&[
    "crates/vpdf-pdf/src/fonts.rs",
    "crates/vpdf-pdf/src/document/mod.rs",
    "crates/vpdf-pdf/src/document/types.rs",
    "crates/vpdf-pdf/src/document/extraction.rs",
    "crates/vpdf-pdf/src/document/metadata.rs",
    "crates/vpdf-pdf/src/document/annotations.rs",
    "crates/vpdf-search/src/search.rs",
    "crates/vpdf-search/src/search/codec.rs",
    "crates/vpdf-search/src/search/normalize.rs",
    "crates/vpdf-search/src/search/join.rs",
    "crates/vpdf-search/src/search/words.rs",
    "crates/vpdf-model/src/text.rs",
    "crates/vpdf-model/src/document.rs",
    "crates/vpdf-model/src/geometry.rs",
  ]);
  println!("cargo::rustc-env=VPDF_INDEX_CODE={code:016x}");
}
