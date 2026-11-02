pub fn version(
  package: &str,
  tag: Option<&str>,
  revision: Option<&str>,
  date: Option<&str>,
  dirty: bool,
) -> String {
  if let Some(tag) = tag {
    assert_eq!(
      tag,
      format!("v{package}"),
      "Release tag differs from Cargo version"
    );
    if !dirty {
      return package.to_owned();
    }
  }
  match (revision, date) {
    (Some(revision), Some(date)) => {
      format!(
        "{package}-dev+{revision} ({date}){}",
        if dirty { " [dirty]" } else { "" }
      )
    }
    _ => format!("{package} (source)"),
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
    for (tag, revision, date, dirty, expected) in [
      (Some("v0.1.0"), Some("abc"), Some("2026-11-02"), false, "0.1.0"),
      (
        Some("v0.1.0"),
        Some("abc"),
        Some("2026-11-02"),
        true,
        "0.1.0-dev+abc (2026-11-02) [dirty]",
      ),
      (
        None,
        Some("abc"),
        Some("2026-11-02"),
        false,
        "0.1.0-dev+abc (2026-11-02)",
      ),
      (None, None, None, false, "0.1.0 (source)"),
    ] {
      assert_eq!(version("0.1.0", tag, revision, date, dirty), expected);
    }
  }

  #[test]
  #[should_panic(expected = "Release tag differs")]
  fn mismatched_tag() {
    version("0.1.0", Some("v0.2.0"), None, None, false);
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
