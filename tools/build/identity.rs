fn releases(tags: &str) -> impl Iterator<Item = semver::Version> + '_ {
  tags
    .lines()
    .filter_map(|tag| semver::Version::parse(tag.strip_prefix('v')?).ok())
}

pub fn release(tags: &str) -> Option<semver::Version> {
  releases(tags).max()
}

pub fn version(package: &str, exact: &str, reachable: &str) -> String {
  if let Some(version) = release(exact) {
    return version.to_string();
  }
  let next = releases(reachable).max().map_or_else(
    || package.to_owned(),
    |version| format!("{}.{}.0", version.major, version.minor + 1),
  );
  format!("{next}-dev")
}

pub fn changelog(
  directory: &std::path::Path,
  release: Option<&str>,
  package: &str,
) -> std::io::Result<String> {
  let version = release.unwrap_or(package);
  match std::fs::read_to_string(directory.join(format!("v{version}.md"))) {
    Ok(markdown) => Ok(notes(version, &markdown)),
    Err(error)
      if release.is_some() && error.kind() == std::io::ErrorKind::NotFound =>
    {
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
    for (exact, reachable, expected) in [
      ("v0.0.1", "v0.0.1", "0.0.1"),
      ("v0.1.0-alpha", "v0.0.1", "0.1.0-alpha"),
      ("v0.1.0-beta", "v0.0.1", "0.1.0-beta"),
      ("v0.1.0-beta.2", "v0.0.1", "0.1.0-beta.2"),
      ("v0.1.0-rc.1", "v0.0.1", "0.1.0-rc.1"),
      ("v0.1.0-beta.2\nv0.1.0-beta.10", "", "0.1.0-beta.10"),
      ("v0.1.0-rc.1\nv0.1.0", "", "0.1.0"),
      ("", "v0.1.0-beta", "0.2.0-dev"),
      ("v2.3.4-rc.1+build.2", "", "2.3.4-rc.1+build.2"),
      ("dev\nv0.0.1\nv0.0.2", "", "0.0.2"),
      ("", "v0.0.1", "0.1.0-dev"),
      ("dev", "v1.9.8\nv1.10.2\nv1.8.0", "1.11.0-dev"),
      ("", "v2.3.4-rc.1", "2.4.0-dev"),
      ("v01.2.3\nv1.2\nv1.2.3bad\n1.2.3", "dev", "0.1.0-dev"),
      ("", "", "0.1.0-dev"),
    ] {
      assert_eq!(version("0.1.0", exact, reachable), expected);
    }
  }

  #[test]
  fn release_notes() -> std::io::Result<()> {
    let root = tempfile::tempdir()?;
    let current = "# Vpdf v0.1.0\n\nNext release";
    std::fs::write(root.path().join("v0.1.0.md"), current)?;
    assert_eq!(changelog(root.path(), None, "0.1.0")?, current);
    assert_eq!(changelog(root.path(), Some("0.0.1"), "0.1.0")?, "");
    let previous = "# Vpdf v0.0.1\n\nFirst release";
    std::fs::write(root.path().join("v0.0.1.md"), previous)?;
    assert_eq!(changelog(root.path(), Some("0.0.1"), "0.1.0")?, previous);
    assert!(changelog(root.path(), None, "0.2.0").is_err());
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
