use super::{Bounds, OnceCell, PageId, Text};

#[derive(Clone, Debug)]
pub struct PageInfo {
  pub bounds: Bounds,
  /// Read on first use through `Document::label`: reading every label
  /// can cost more than the rest of opening a long manual.
  pub label: OnceCell<String>,
}

#[derive(Clone, Debug)]
pub struct Link {
  first: Bounds,
  /// Lines or columns it wraps onto.
  rest: Vec<Bounds>,
  pub target: Target,
}

impl Link {
  pub fn new(first: Bounds, target: Target) -> Self {
    Self { first, rest: Vec::new(), target }
  }

  pub fn first(&self) -> Bounds {
    self.first
  }

  pub fn last(&self) -> Bounds {
    self.rest.last().copied().unwrap_or(self.first)
  }

  /// Every rectangle, in reading order.
  pub fn boxes(&self) -> impl Iterator<Item = Bounds> + '_ {
    std::iter::once(self.first).chain(self.rest.iter().copied())
  }

  pub fn contains(&self, x: f32, y: f32) -> bool {
    self.boxes().any(|bounds| bounds.contains(x, y))
  }

  /// Its text; wrapped parts join unspaced.
  pub fn anchor(&self, text: &Text, page: PageId) -> String {
    self.boxes().map(|bounds| text.in_areas(page, &[bounds]).1).collect()
  }

  /// Takes `part`'s boxes after its own.
  pub(crate) fn append(&mut self, part: Self) {
    self.rest.push(part.first);
    self.rest.extend(part.rest);
  }

  /// Takes `part`'s boxes before its own.
  pub(crate) fn prepend(&mut self, part: Self) {
    let own = std::mem::replace(&mut self.rest, part.rest);
    self.rest.push(std::mem::replace(&mut self.first, part.first));
    self.rest.extend(own);
  }

  /// Adds a box after its own.
  pub(crate) fn push(&mut self, bounds: Bounds) {
    self.rest.push(bounds);
  }

  /// Heap bytes beyond the struct.
  pub(crate) fn weight(&self) -> usize {
    let uri = match &self.target {
      Target::Uri(uri) => uri.capacity(),
      Target::Page(_) | Target::Point { .. } => 0,
    };
    self.rest.capacity() * size_of::<Bounds>() + uri
  }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
  Page(PageId),
  Point { page: PageId, x: f32, y: f32 },
  Uri(String),
}

impl From<mupdf::link::LinkDestination> for Target {
  fn from(dest: mupdf::link::LinkDestination) -> Self {
    use mupdf::DestinationKind as D;
    let page = PageId(dest.loc.page_number as usize);
    let (x, y) = match dest.kind {
      D::XYZ { left, top, .. } => (left, top),
      D::FitH { top } | D::FitBH { top } => (None, top),
      D::FitV { left } | D::FitBV { left } => (left, None),
      _ => return Self::Page(page),
    };
    if x.is_none() && y.is_none() {
      return Self::Page(page);
    }
    Self::Point {
      page,
      x: x.filter(|v| v.is_finite()).unwrap_or(0.0),
      y: y.filter(|v| v.is_finite()).unwrap_or(0.0),
    }
  }
}

/// A reader-visible annotation: markup, note, or shape.
#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
  pub kind: String,
  /// Marked areas; markup has one per quad.
  pub areas: Vec<Bounds>,
  pub contents: String,
  pub author: String,
}

#[derive(Clone, Debug)]
pub struct Heading {
  pub title: String,
  pub depth: usize,
  pub target: Target,
}
