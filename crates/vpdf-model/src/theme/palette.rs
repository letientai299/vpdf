use serde::{Deserialize, Deserializer};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Palette {
  #[serde(deserialize_with = "hex")]
  pub background: [u8; 3],
  #[serde(deserialize_with = "hex")]
  pub foreground: [u8; 3],
  #[serde(deserialize_with = "hex")]
  pub surface: [u8; 3],
  #[serde(deserialize_with = "hex")]
  pub cursorline: [u8; 3],
  #[serde(deserialize_with = "hex")]
  pub muted: [u8; 3],
  #[serde(deserialize_with = "hex")]
  pub accent: [u8; 3],
  #[serde(deserialize_with = "hex")]
  pub highlight: [u8; 3],
  #[serde(deserialize_with = "hex")]
  pub r#match: [u8; 3],
  #[serde(deserialize_with = "hex")]
  pub link: [u8; 3],
  #[serde(deserialize_with = "hex")]
  pub error: [u8; 3],
}

fn hex<'de, D: Deserializer<'de>>(
  deserializer: D,
) -> Result<[u8; 3], D::Error> {
  let text = String::deserialize(deserializer)?;
  let digits = text
    .strip_prefix('#')
    .filter(|digits| {
      digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
    .ok_or_else(|| serde::de::Error::custom("expected #rrggbb"))?;
  let value =
    u32::from_str_radix(digits, 16).map_err(serde::de::Error::custom)?;
  Ok([(value >> 16) as u8, (value >> 8) as u8, value as u8])
}

impl Default for Palette {
  fn default() -> Self {
    Self::dark()
  }
}

pub fn is_light(rgb: [u8; 3]) -> bool {
  // Prefer higher black/white text contrast.
  let crossover = (1.05_f64 * 0.05).sqrt() - 0.05;
  luminance(rgb) > crossover
}

pub fn luminance(rgb: [u8; 3]) -> f64 {
  let linear = rgb.map(|channel| {
    let value = f64::from(channel) / 255.0;
    if value <= 0.04045 {
      value / 12.92
    } else {
      ((value + 0.055) / 1.055).powf(2.4)
    }
  });
  linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722
}

impl Palette {
  pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
    toml::from_str(text)
  }

  /// # Panics
  ///
  /// Invalid embedded light palette.
  pub fn light() -> Self {
    static PALETTE: OnceLock<Palette> = OnceLock::new();
    *PALETTE.get_or_init(|| {
      Self::parse(include_str!("../../../../etc/themes/light.toml"))
        .expect("invalid built-in light palette")
    })
  }

  /// # Panics
  ///
  /// Invalid embedded dark palette.
  pub fn dark() -> Self {
    static PALETTE: OnceLock<Palette> = OnceLock::new();
    *PALETTE.get_or_init(|| {
      Self::parse(include_str!("../../../../etc/themes/dark.toml"))
        .expect("invalid built-in dark palette")
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn repository_palettes_parse() {
    let folder =
      std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../etc/themes");
    for entry in std::fs::read_dir(folder).unwrap() {
      let path = entry.unwrap().path();
      if path.extension().is_some_and(|ext| ext == "toml") {
        let text = std::fs::read_to_string(&path).unwrap();
        Palette::parse(&text)
          .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
      }
    }
  }

  #[test]
  fn tokens_are_required_hex() {
    let text = include_str!("../../../../etc/themes/dark.toml");
    for line in text.lines().filter(|line| line.contains(" = ")) {
      assert!(Palette::parse(&text.replace(line, "")).is_err(), "{line}");
      let (key, _) = line.split_once(" = ").unwrap();
      for invalid in
        ["red", "#xyzxyz", "#12345", "#1234567", "123456", "#１２３"]
      {
        assert!(
          Palette::parse(&text.replace(line, &format!("{key} = {invalid:?}")))
            .is_err(),
          "{key}: {invalid}"
        );
      }
    }
  }
}
