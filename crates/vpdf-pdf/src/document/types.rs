use super::{Bounds, OnceCell, PageId, Target};

#[derive(Clone, Debug)]
pub struct PageInfo {
  pub bounds: Bounds,
  /// Read on first use through `Document::label`: reading every label
  /// can cost more than the rest of opening a long manual.
  pub label: OnceCell<String>,
}

pub(crate) fn target(dest: mupdf::link::LinkDestination) -> Target {
  use mupdf::DestinationKind as D;
  let page = PageId(dest.loc.page_number as usize);
  let (x, y) = match dest.kind {
    D::XYZ { left, top, .. } => (left, top),
    D::FitH { top } | D::FitBH { top } => (None, top),
    D::FitV { left } | D::FitBV { left } => (left, None),
    _ => return Target::Page(page),
  };
  if x.is_none() && y.is_none() {
    return Target::Page(page);
  }
  Target::Point {
    page,
    x: x.filter(|v| v.is_finite()).unwrap_or(0.0),
    y: y.filter(|v| v.is_finite()).unwrap_or(0.0),
  }
}
