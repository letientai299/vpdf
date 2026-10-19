use crate::text::PageId;
use std::cmp::Ordering;
use vpdf_model::document::Target;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
  pub page: PageId,
  pub x: f32,
  pub y: f32,
}

impl Point {
  pub fn from_target(target: &Target) -> Option<Self> {
    match *target {
      Target::Page(page) => Some(Self { page, x: 0.0, y: 0.0 }),
      Target::Point { page, x, y } => Some(Self { page, x, y }),
      Target::Uri(_) => None,
    }
  }

  pub fn target(self) -> Target {
    Target::Point { page: self.page, x: self.x, y: self.y }
  }

  pub fn compare(self, other: Self) -> Ordering {
    self
      .page
      .cmp(&other.page)
      .then(self.y.total_cmp(&other.y))
      .then(self.x.total_cmp(&other.x))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn reading_order_keeps_coordinate_axes() {
    let point = Point { page: PageId(1), x: 40.0, y: 20.0 };
    for (other, expected) in [
      (Point { page: PageId(2), x: 0.0, y: 0.0 }, Ordering::Less),
      (Point { x: 0.0, y: 30.0, ..point }, Ordering::Less),
      (Point { x: 50.0, ..point }, Ordering::Less),
      (point, Ordering::Equal),
    ] {
      assert_eq!(point.compare(other), expected);
      assert_eq!(other.compare(point), expected.reverse());
    }
    let zero = Point { x: 0.0, y: 0.0, ..point };
    assert_eq!(zero.compare(Point { x: -0.0, ..zero }), Ordering::Greater);
    assert_eq!(zero.compare(Point { y: f32::NAN, ..zero }), Ordering::Less);
    assert_eq!(Point::from_target(&point.target()), Some(point));
    assert_eq!(Point::from_target(&Target::Page(point.page)), Some(zero));
    assert_eq!(Point::from_target(&Target::Uri(String::new())), None);
  }
}
