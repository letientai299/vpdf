fn releases(tags: &str) -> impl Iterator<Item = semver::Version> + '_ {
  tags
    .lines()
    .filter_map(|tag| semver::Version::parse(tag.strip_prefix('v')?).ok())
}

pub fn release(tags: &str) -> Option<semver::Version> {
  releases(tags).max()
}

/// A tagged release's version; other builds are `dev`.
pub fn version(exact: &str) -> String {
  release(exact).map_or_else(|| "dev".to_owned(), |version| version.to_string())
}

pub fn changelog(
  directory: &std::path::Path,
  release: Option<&str>,
) -> std::io::Result<String> {
  let Some(version) = release else { return Ok(String::new()) };
  match std::fs::read_to_string(directory.join(format!("v{version}.md"))) {
    Ok(markdown) => Ok(notes(version, &markdown)),
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
      Ok(String::new())
    }
    Err(error) => Err(error),
  }
}

pub fn notes(package: &str, markdown: &str) -> String {
  let (heading, body) =
    markdown.split_once('\n').expect("Changelog heading missing");
  assert_eq!(
    heading.trim(),
    format!("# Vpdf v{package}"),
    "Changelog version differs"
  );
  assert!(!body.trim().is_empty(), "Changelog content missing");
  markdown.trim().to_owned()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn source_versions() {
    for (exact, expected) in [
      ("v0.0.1", "0.0.1"),
      ("v0.1.0-alpha", "0.1.0-alpha"),
      ("v0.1.0-beta.2", "0.1.0-beta.2"),
      ("v0.1.0-rc.1", "0.1.0-rc.1"),
      ("v0.1.0-beta.2\nv0.1.0-beta.10", "0.1.0-beta.10"),
      ("v0.1.0-rc.1\nv0.1.0", "0.1.0"),
      ("v2.3.4-rc.1+build.2", "2.3.4-rc.1+build.2"),
      ("dev\nv0.0.1\nv0.0.2", "0.0.2"),
      ("dev", "dev"),
      ("v01.2.3\nv1.2\nv1.2.3bad\n1.2.3", "dev"),
      ("", "dev"),
    ] {
      assert_eq!(version(exact), expected);
    }
  }

  #[test]
  fn release_notes() -> std::io::Result<()> {
    let root = tempfile::tempdir()?;
    let notes = "# Vpdf v0.0.1\n\nFirst release";
    std::fs::write(root.path().join("v0.0.1.md"), notes)?;
    assert_eq!(changelog(root.path(), None)?, "");
    assert_eq!(changelog(root.path(), Some("0.0.2"))?, "");
    assert_eq!(changelog(root.path(), Some("0.0.1"))?, notes);
    Ok(())
  }

  #[test]
  fn bundled_notes() {
    assert_eq!(
      notes("0.1.0", "# Vpdf v0.1.0\n\nFeatures\n"),
      "# Vpdf v0.1.0\n\nFeatures"
    );
  }

  #[test]
  #[should_panic(expected = "Changelog content missing")]
  fn empty_notes() {
    notes("0.1.0", "# Vpdf v0.1.0\n\n");
  }

  #[test]
  #[should_panic(expected = "Changelog version differs")]
  fn mismatched_notes() {
    notes("0.2.0", "# Vpdf v0.1.0\n\nFeatures");
  }
}
