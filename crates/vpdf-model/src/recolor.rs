use serde::{Deserialize, Deserializer, Serialize};

pub const DEFAULT_AMOUNT: u8 = 60;
pub const MAX_AMOUNT: u8 = 100;

#[derive(
  Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize,
)]
#[serde(rename_all = "lowercase")]
enum Preset {
  #[default]
  Off,
  Dark,
  Sepia,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize)]
pub struct Recolor {
  preset: Preset,
  amount: u8,
}

impl Recolor {
  fn new(preset: Preset, amount: u64) -> Self {
    if amount == 0 || preset == Preset::Off {
      return Self::default();
    }
    Self { preset, amount: amount.min(u64::from(MAX_AMOUNT)) as u8 }
  }

  pub fn parse(argument: &str) -> Option<Self> {
    let mut words = argument.split_whitespace();
    let preset = match words.next() {
      None => return Some(Self::new(Preset::Dark, DEFAULT_AMOUNT.into())),
      Some("off") => return words.next().is_none().then(Self::default),
      Some("dark") => Preset::Dark,
      Some("sepia") => Preset::Sepia,
      _ => return None,
    };
    let amount = match words.next() {
      None => DEFAULT_AMOUNT,
      Some(word) => {
        if !word.bytes().all(|byte| byte.is_ascii_digit()) {
          return None;
        }
        word.bytes().fold(0_u8, |amount, byte| {
          amount.saturating_mul(10).saturating_add(byte - b'0').min(MAX_AMOUNT)
        })
      }
    };
    words.next().is_none().then(|| Self::new(preset, amount.into()))
  }

  pub fn apply(self, pixels: &mut [u8]) {
    self.apply_pixels(pixels.as_chunks_mut::<3>().0);
  }

  pub fn apply_rgba(self, pixels: &mut [u8]) {
    self.apply_pixels(pixels.as_chunks_mut::<4>().0);
  }

  fn apply_pixels<const N: usize>(self, pixels: &mut [[u8; N]]) {
    if self == Self::default() {
      return;
    }
    let strength = u32::from(self.amount);
    let lerp =
      |from: u32, to: u32| (from * (100 - strength) + to * strength + 50) / 100;
    let (ink, paper) = match self.preset {
      Preset::Off => return,
      Preset::Dark => ([lerp(224, 255); 3], [lerp(64, 0); 3]),
      Preset::Sepia => (
        [lerp(0, 48), lerp(0, 36), lerp(0, 24)],
        [lerp(255, 238), lerp(255, 220), lerp(255, 185)],
      ),
    };
    let tables: [[u8; 256]; 3] = std::array::from_fn(|channel| {
      std::array::from_fn(|value| {
        let value = value as u32;
        ((ink[channel] * (255 - value) + paper[channel] * value + 127) / 255)
          as u8
      })
    });
    for pixel in pixels {
      for channel in 0..3 {
        pixel[channel] = tables[channel][usize::from(pixel[channel])];
      }
    }
  }
}

impl<'de> Deserialize<'de> for Recolor {
  fn deserialize<D: Deserializer<'de>>(
    deserializer: D,
  ) -> Result<Self, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Saved {
      Setting { preset: Preset, amount: u64 },
      Other(serde::de::IgnoredAny),
    }
    Ok(match Saved::deserialize(deserializer)? {
      Saved::Setting { preset, amount } => Self::new(preset, amount),
      Saved::Other(_) => Self::default(),
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn commands_normalize() {
    assert_eq!(Recolor::parse(""), Recolor::parse("dark 60"));
    for preset in ["dark", "sepia"] {
      assert_eq!(
        Recolor::parse(preset),
        Recolor::parse(&format!("{preset} 60"))
      );
      assert_eq!(
        Recolor::parse(&format!("{preset} 0")),
        Some(Recolor::default())
      );
      for amount in ["100", "101", "999999999999999999999999999999999999"] {
        assert_eq!(
          Recolor::parse(&format!("{preset} {amount}")),
          Recolor::parse(&format!("{preset} 100"))
        );
      }
    }
    assert_eq!(Recolor::parse("off"), Some(Recolor::default()));
    for invalid in [
      "60",
      "invert",
      "dark -1",
      "dark +1",
      "sepia 1.5",
      "off 0",
      "dark 1 x",
      "dark 1a",
    ] {
      assert_eq!(Recolor::parse(invalid), None, "{invalid}");
    }
  }

  #[test]
  fn exact_pixels() {
    let original = vec![0, 0, 0, 255, 255, 255, 128, 128, 128, 255, 0, 128];
    let mut pixels = original.clone();
    let pointer = pixels.as_ptr();
    Recolor::default().apply(&mut pixels);
    assert_eq!(pixels, original);
    Recolor::parse("dark 100").unwrap().apply(&mut pixels);
    assert_eq!(pixels, [255, 255, 255, 0, 0, 0, 127, 127, 127, 0, 255, 127]);
    assert_eq!(pointer, pixels.as_ptr());
    pixels.copy_from_slice(&original);
    Recolor::parse("sepia 100").unwrap().apply(&mut pixels);
    assert_eq!(
      pixels,
      [48, 36, 24, 238, 220, 185, 143, 128, 105, 238, 36, 105]
    );
  }

  #[test]
  fn rgba_preserves_alpha() {
    for command in ["off", "dark 60", "sepia 30"] {
      let recolor = Recolor::parse(command).unwrap();
      let mut rgb = [0, 128, 255, 255, 0, 128];
      let mut rgba = [0, 128, 255, 0, 255, 0, 128, 173];
      recolor.apply(&mut rgb);
      recolor.apply_rgba(&mut rgba);
      assert_eq!(&rgba[..3], &rgb[..3]);
      assert_eq!(&rgba[4..7], &rgb[3..]);
      assert_eq!((rgba[3], rgba[7]), (0, 173));
    }
  }

  #[test]
  fn strength_endpoints() {
    let mut previous = [0, 0, 0, 255, 255, 255];
    for amount in 1..=100 {
      let mut pixels = [0, 0, 0, 255, 255, 255];
      Recolor::parse(&format!("dark {amount}")).unwrap().apply(&mut pixels);
      assert!(pixels[0] > pixels[3]);
      assert!(pixels[0] >= previous[0]);
      assert!(pixels[3] <= previous[3]);
      previous = pixels;
      let mut sepia = [0, 0, 0, 255, 255, 255];
      Recolor::parse(&format!("sepia {amount}")).unwrap().apply(&mut sepia);
      assert!((0..3).all(|channel| sepia[channel] < sepia[channel + 3]));
    }
  }

  #[test]
  fn saved_values_normalize() {
    for value in [
      "null",
      "42",
      "\"dark\"",
      "{}",
      "{\"preset\":\"unknown\",\"amount\":60}",
      "{\"preset\":\"dark\",\"amount\":-1}",
    ] {
      assert_eq!(
        serde_json::from_str::<Recolor>(value).unwrap(),
        Recolor::default()
      );
    }
    for command in ["off", "dark 60", "sepia 30"] {
      let value = Recolor::parse(command).unwrap();
      assert_eq!(
        serde_json::from_str::<Recolor>(
          &serde_json::to_string(&value).unwrap()
        )
        .unwrap(),
        value
      );
    }
    assert_eq!(
      serde_json::from_str::<Recolor>(r#"{"preset":"dark","amount":1000}"#)
        .unwrap(),
      Recolor::parse("dark 100").unwrap()
    );
  }
}
