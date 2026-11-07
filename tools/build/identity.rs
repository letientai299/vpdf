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
}
