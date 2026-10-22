use crate::text::{Bounds, PageId, Position, Text};
use std::path::PathBuf;

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
  pub fn append(&mut self, part: Self) {
    self.rest.push(part.first);
    self.rest.extend(part.rest);
  }

  /// Takes `part`'s boxes before its own.
  pub fn prepend(&mut self, part: Self) {
    let own = std::mem::replace(&mut self.rest, part.rest);
    self.rest.push(std::mem::replace(&mut self.first, part.first));
    self.rest.extend(own);
  }

  /// Adds a box after its own.
  pub fn push(&mut self, bounds: Bounds) {
    self.rest.push(bounds);
  }

  /// Heap bytes beyond the struct.
  pub fn weight(&self) -> usize {
    let uri = match &self.target {
      Target::Uri(uri) => uri.capacity(),
      Target::Page(_) | Target::Point { .. } => 0,
    };
    self.rest.capacity() * size_of::<Bounds>() + uri
  }
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum Target {
  Page(PageId),
  Point { page: PageId, x: f32, y: f32 },
  Uri(String),
}

/// A reader-visible annotation: markup, note, or shape.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Annotation {
  pub kind: String,
  /// Marked areas; markup has one per quad.
  pub areas: Vec<Bounds>,
  pub contents: String,
  pub author: String,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct Heading {
  pub title: String,
  pub depth: usize,
  pub target: Target,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct Figure {
  pub bounds: Bounds,
  pub caption: String,
  pub source: FigureSource,
}

/// What a figure was found from.
#[derive(
  Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize,
)]
pub enum FigureSource {
  /// Image or vector blocks.
  Graphic,
  /// A caption line alone.
  Caption,
}

pub type Thumbnail = crate::image::Rgb;

pub struct Facts {
  pub pages: usize,
  pub bytes: u64,
  pub metadata: Vec<(String, String)>,
  pub section: Option<String>,
  pub thumb: Option<Thumbnail>,
}

/// Pixel box a thumbnail fits; none without graphics.
pub type Fit = Option<(u32, u32)>;

/// A recent document whose facts and thumbnail to load.
pub struct FactsRequest {
  pub path: PathBuf,
  pub cursor: Position,
  pub rotation: u16,
  pub fit: Fit,
}

/// Facts loaded for a request's path and fit.
pub struct LoadedFacts {
  pub fit: Fit,
  pub facts: Result<Facts, String>,
}

/// Last outline entry starting at or before `page`.
pub fn section(outline: &[Heading], page: PageId, y: f32) -> Option<usize> {
  outline.iter().rposition(|heading| match heading.target {
    Target::Page(start) => start <= page,
    Target::Point { page: start, y: top, .. } => {
      start < page || start == page && top <= y
    }
    Target::Uri(_) => false,
  })
}
