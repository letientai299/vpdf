use crate::style::Color;
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
  #[serde(default)]
  pub mode: ModeFills,
}

/// Badge fills; unset keys use tokens.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, expecting = "a [mode] table of #rrggbb fills")]
pub struct ModeFills {
  #[serde(default, deserialize_with = "some_hex")]
  pub normal: Option<[u8; 3]>,
  #[serde(default, deserialize_with = "some_hex")]
  pub cursor: Option<[u8; 3]>,
  #[serde(default, deserialize_with = "some_hex")]
  pub visual: Option<[u8; 3]>,
  #[serde(default, deserialize_with = "some_hex")]
  pub transient: Option<[u8; 3]>,
}

fn some_hex<'de, D: Deserializer<'de>>(
  deserializer: D,
) -> Result<Option<[u8; 3]>, D::Error> {
  hex(deserializer).map(Some)
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

/// WCAG AA minimum for ordinary text.
pub const TEXT_CONTRAST: f64 = 4.5;

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

/// WCAG contrast ratio of two colors.
pub fn contrast(first: [u8; 3], second: [u8; 3]) -> f64 {
  let (first, second) = (luminance(first), luminance(second));
  (first.max(second) + 0.05) / (first.min(second) + 0.05)
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

  /// Text and fill colors of a `color` badge.
  pub fn badge(self, color: Color) -> ([u8; 3], [u8; 3]) {
    let (fill, token) = match color {
      Color::Blue => (self.mode.normal, self.accent),
      Color::Cyan => (self.mode.cursor, self.link),
      Color::Magenta => (self.mode.visual, self.highlight),
      Color::Yellow => (self.mode.transient, self.r#match),
    };
    let fill = fill.unwrap_or(token);
    (self.text_on(fill), fill)
  }

  /// `background`, or black or white when unreadable.
  pub fn text_on(self, fill: [u8; 3]) -> [u8; 3] {
    // Black or white always reaches 4.5:1.
    if contrast(self.background, fill) >= TEXT_CONTRAST {
      self.background
    } else if is_light(fill) {
      [0; 3]
    } else {
      [255; 3]
    }
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
  fn mode_fills_override_badges() {
    let base = include_str!("../../../../etc/themes/dark.toml");
    let dark = Palette::parse(base).unwrap();
    assert_eq!(dark.badge(Color::Blue).1, dark.accent);
    let palette = Palette::parse(&format!(
      "{base}\n[mode]\nnormal = \"#ff0000\"\nvisual = \"#101010\"\n"
    ))
    .unwrap();
    let light = include_str!("../../../../etc/themes/light.toml");
    let light =
      Palette::parse(&format!("{light}\n[mode]\nvisual = \"#ffff00\"\n"))
        .unwrap();
    // Background text, else black or white.
    for (palette, color, text, fill) in [
      (palette, Color::Blue, dark.background, [255, 0, 0]),
      (palette, Color::Magenta, [255; 3], [16; 3]),
      (palette, Color::Cyan, dark.badge(Color::Cyan).0, dark.link),
      (palette, Color::Yellow, dark.badge(Color::Yellow).0, dark.r#match),
      (light, Color::Magenta, [0; 3], [255, 255, 0]),
    ] {
      assert_eq!(palette.badge(color), (text, fill), "{color:?}");
    }
    let error = Palette::parse(&format!("{base}\nmode = \"red\"\n"))
      .unwrap_err()
      .to_string();
    assert!(error.contains("[mode] table"), "{error}");
    for invalid in ["[mode]\nnormal = \"red\"", "[mode]\nyank = \"#ffffff\""] {
      assert!(Palette::parse(&format!("{base}\n{invalid}\n")).is_err());
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
