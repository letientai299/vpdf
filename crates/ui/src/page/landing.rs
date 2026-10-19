use crate::text::Position;
use std::time::{Duration, Instant};
use vpdf_model::document::Target;
use vpdf_reader::reader::Reader;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Span {
  Line,
  Glyph,
}

struct Flash {
  position: Position,
  until: Instant,
  span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landed {
  target: Position,
  scroll: f32,
  pan: f32,
  text_top: Position,
}

impl Landed {
  pub fn new(
    target: Position,
    scroll: f32,
    pan: f32,
    text_top: Position,
  ) -> Self {
    Self { target, scroll, pan, text_top }
  }
}

#[derive(Default)]
pub struct Landing {
  flash: Option<Flash>,
  landed: Option<Landed>,
  aimed: Option<(Landed, Target)>,
}

impl Landing {
  pub const DURATION: Duration = Duration::from_millis(600);

  pub fn show(&mut self, position: Position, span: Span, now: Instant) {
    self.flash = Some(Flash { position, span, until: now + Self::DURATION });
  }

  pub fn clear_flash(&mut self) {
    self.flash = None;
  }

  pub fn position(&self) -> Option<Position> {
    self.flash.as_ref().map(|flash| flash.position)
  }

  pub fn deadline(&self) -> Option<Instant> {
    self.flash.as_ref().map(|flash| flash.until)
  }

  pub fn expire(&mut self, now: Instant) -> bool {
    if self.deadline().is_some_and(|until| now >= until) {
      self.clear_flash();
      return true;
    }
    false
  }

  pub fn line(&self) -> Option<Position> {
    let flash = self.flash.as_ref()?;
    (flash.span == Span::Line).then_some(flash.position)
  }

  pub fn glyph(&self, reader: &Reader) -> Option<Position> {
    let flash = self.flash.as_ref()?;
    let normal = reader.viewport_mode() && reader.selection.is_none();
    (flash.span == Span::Glyph && normal).then_some(flash.position)
  }

  pub fn record(&mut self, view: Landed) {
    self.landed = Some(view);
  }

  pub fn matches(&self, view: Landed) -> bool {
    self.landed == Some(view)
  }

  pub fn aim(&mut self, target: Target) {
    self.aimed = self.landed.map(|view| (view, target));
  }

  pub fn aimed(&self) -> Option<&Target> {
    let (view, target) = self.aimed.as_ref()?;
    (self.landed == Some(*view)).then_some(target)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::text::PageId;
  use vpdf_reader::reader::{Operator, SelectionKind};

  #[test]
  fn flash_expiry_preserves_view_identity() {
    let now = Instant::now();
    let position = Position { line: 3, ..Position::default() };
    let view = Landed::new(position, 100.0, 4.0, Position::default());
    let target = Target::Page(PageId(1));
    let mut landing = Landing::default();
    landing.record(view);
    landing.aim(target.clone());
    landing.show(position, Span::Line, now);
    let until = now + Landing::DURATION;
    assert_eq!(landing.deadline(), Some(until));
    assert!(
      !landing.expire(until.checked_sub(Duration::from_nanos(1)).unwrap())
    );
    assert_eq!(landing.line(), Some(position));
    assert!(landing.expire(until));
    assert!(!landing.expire(until));
    assert_eq!(landing.position(), None);
    assert!(landing.matches(view));
    assert_eq!(landing.aimed(), Some(&target));
    let moved = Landed::new(position, 101.0, 4.0, Position::default());
    landing.record(moved);
    assert!(!landing.matches(view));
    assert_eq!(landing.aimed(), None);
    landing.record(view);
    assert_eq!(landing.aimed(), Some(&target));
  }

  #[test]
  fn glyph_flash_requires_normal_viewport() {
    let mut landing = Landing::default();
    let position = Position::default();
    let mut reader = Reader::default();
    landing.show(position, Span::Glyph, Instant::now());
    assert_eq!(landing.glyph(&reader), Some(position));
    assert_eq!(landing.line(), None);
    reader.enter_cursor();
    assert_eq!(landing.glyph(&reader), None);
    reader.normal();
    reader.operator = Some(Operator { count: 1, register: '"' });
    assert_eq!(landing.glyph(&reader), None);
    reader.operator = None;
    reader.visual(SelectionKind::Line);
    assert_eq!(landing.glyph(&reader), None);
    landing.show(position, Span::Line, Instant::now());
    assert_eq!(landing.line(), Some(position));
    landing.clear_flash();
    assert_eq!(landing.deadline(), None);
  }

  #[test]
  fn view_identity_includes_each_coordinate() {
    let position = Position::default();
    let changed = Position { line: 1, ..position };
    let view = Landed::new(position, 100.0, 4.0, position);
    let mut landing = Landing::default();
    landing.aim(Target::Page(PageId(1)));
    landing.record(view);
    assert_eq!(landing.aimed(), None);
    assert!(landing.matches(view));
    for other in [
      Landed::new(changed, 100.0, 4.0, position),
      Landed::new(position, 101.0, 4.0, position),
      Landed::new(position, 100.0, 5.0, position),
      Landed::new(position, 100.0, 4.0, changed),
    ] {
      assert!(!landing.matches(other));
    }
  }
}
