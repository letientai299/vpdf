use super::Overlay;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(transparent)]
pub struct UnitInterval(f64);

// Construction excludes NaN.
impl Eq for UnitInterval {}

impl TryFrom<f64> for UnitInterval {
  type Error = &'static str;

  fn try_from(value: f64) -> Result<Self, Self::Error> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
      Ok(Self(value))
    } else {
      Err("value outside unit interval")
    }
  }
}

impl UnitInterval {
  pub const fn get(self) -> f64 {
    self.0
  }
}

macro_rules! opacity {
  ($($name:ident => $overlay:ident),+ $(,)?) => {
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
    pub struct Opacity {
      $(#[serde(skip_serializing_if = "Option::is_none")]
      pub $name: Option<UnitInterval>,)+
    }

    impl Opacity {
      pub fn field(&mut self, name: &str) -> Option<&mut Option<UnitInterval>> {
        match name {
          $(stringify!($name) => Some(&mut self.$name),)+
          _ => None,
        }
      }

      pub fn values(self) -> impl Iterator<Item = (&'static str, f64)> {
        [$((stringify!($name), self.$name.map_or(
          f64::from(Overlay::$overlay.default_alpha()) / 255.0,
          UnitInterval::get,
        )),)+].into_iter()
      }
    }
  };
}

opacity! {
  selection => Selection,
  search => Match,
  cursor => Cursor,
  text_cursor => TextCursor,
  cursorline => Cursorline,
  link => Link,
  veil => PaneVeil,
  ring => Ring,
  scrollbar => Track,
  inactive => Inactive,
}
