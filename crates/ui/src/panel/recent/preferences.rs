use serde::{Deserialize, Serialize};

#[derive(
  Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize,
)]
#[serde(default)]
pub struct Preferences {
  pub(super) beside: Option<Ratio>,
  pub(super) below: Option<Ratio>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "u16", into = "u16")]
pub(super) struct Ratio(u16);

impl Ratio {
  pub fn new(position: u16, span: u16) -> Self {
    Self(
      ((u32::from(position.min(span)) * 1000 + u32::from(span) / 2)
        / u32::from(span.max(1))) as u16,
    )
  }

  pub fn cells(self, span: u16) -> u16 {
    ((u32::from(span) * u32::from(self.0) + 500) / 1000) as u16
  }
}

impl TryFrom<u16> for Ratio {
  type Error = &'static str;

  fn try_from(value: u16) -> Result<Self, Self::Error> {
    if value <= 1000 {
      Ok(Self(value))
    } else {
      Err("Divider ratio exceeds one")
    }
  }
}

impl From<Ratio> for u16 {
  fn from(value: Ratio) -> Self {
    value.0
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn ratios_validate_and_roundtrip() -> anyhow::Result<()> {
    let preferences = Preferences {
      beside: Some(Ratio::new(60, 120)),
      below: Some(Ratio::new(20, 40)),
    };
    assert_eq!(
      serde_json::from_str::<Preferences>(&serde_json::to_string(
        &preferences
      )?)?,
      preferences
    );
    for invalid in [r#"{"beside":1001}"#, r#"{"beside":-1}"#] {
      assert!(serde_json::from_str::<Preferences>(invalid).is_err());
    }
    assert_eq!(Ratio::new(u16::MAX, u16::MAX).cells(u16::MAX), u16::MAX);
    Ok(())
  }
}
