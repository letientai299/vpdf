use super::{Frame, View};
use crate::Result;
use crate::layout::Layout;
use crate::page::{Viewport, scroll::Scroll};
use crate::text::Position;
use crate::text_view::TextView;
use std::ops::Deref;
use vpdf_model::geometry::Screen;
use vpdf_reader::reader::{
  JumpDescription, Location, Operator, Reader, SelectionKind, Yank,
};
use vpdf_search::{query::Query, search::Hit};

#[derive(Default)]
pub struct Pane {
  reader: Reader,
  tracking: Tracking,
}

#[derive(Clone, Default)]
pub struct Tracking {
  target: Option<Position>,
  animation: Scroll,
  landing: crate::page::landing::Landing,
}

impl Deref for Pane {
  type Target = Reader;

  fn deref(&self) -> &Reader {
    &self.reader
  }
}

impl Pane {
  pub fn new(reader: Reader, tracking: Tracking) -> Self {
    Self { reader, tracking }
  }

  pub fn bind<'a, S>(
    &'a mut self,
    layout: &'a mut Layout,
    text: &'a mut TextView,
    frame: Frame,
    source: S,
  ) -> View<'a, S> {
    View {
      reader: &mut self.reader,
      target: &mut self.tracking.target,
      animation: &mut self.tracking.animation,
      landing: &mut self.tracking.landing,
      layout,
      text,
      frame,
      source,
    }
  }

  pub fn tracking(&self) -> &Tracking {
    &self.tracking
  }

  pub fn target(&self) -> Option<Position> {
    self.tracking.target
  }

  pub fn point_to(&mut self, position: Position) {
    self.tracking.target = Some(position);
  }

  pub fn clear_target(&mut self) {
    self.tracking.target = None;
  }

  pub fn animation(&self) -> &Scroll {
    &self.tracking.animation
  }

  pub fn cancel_scroll(&mut self) {
    self.tracking.animation.cancel();
  }

  pub fn finish_scroll(&mut self) -> Option<f32> {
    self.tracking.animation.finish()
  }

  pub fn animate_to(&mut self, target: f32, now: std::time::Instant) {
    self.tracking.animation.start(target, now);
  }

  pub fn landing(&self) -> &crate::page::landing::Landing {
    &self.tracking.landing
  }

  pub fn flash(
    &mut self,
    position: Position,
    span: crate::page::landing::Span,
    now: std::time::Instant,
  ) {
    self.tracking.landing.show(position, span, now);
  }

  pub fn clear_flash(&mut self) {
    self.tracking.landing.clear_flash();
  }

  pub fn expire_flash(&mut self, now: std::time::Instant) -> bool {
    self.tracking.landing.expire(now)
  }

  pub fn aim(&mut self, target: vpdf_model::document::Target) {
    self.tracking.landing.aim(target);
  }

  pub fn page<'a, S>(
    &'a mut self,
    layout: &'a mut Layout,
    source: S,
    screen: Screen,
  ) -> Viewport<'a, S> {
    Viewport::new(layout, &mut self.reader.scroll, source, screen)
  }

  pub fn animate(&mut self, now: std::time::Instant) -> bool {
    let Some(scroll) = self.tracking.animation.step(self.reader.scroll, now)
    else {
      return false;
    };
    self.reader.scroll = scroll;
    true
  }

  pub fn prepare(&mut self, action: crate::keys::Action) {
    self.tracking.animation.finish_before(action, &mut self.reader);
  }

  pub fn release_bar(&mut self, bar: &mut crate::scrollbar::Bar) -> bool {
    bar.release(&mut self.reader)
  }

  pub fn press_bar(
    &mut self,
    bar: &mut crate::scrollbar::Bar,
    press: crate::scrollbar::Press,
    origin: Location,
  ) -> Option<f32> {
    press.apply(bar, origin, &mut self.reader)
  }

  pub fn notice(&mut self, message: impl Into<String>) {
    self.reader.message = message.into();
  }

  pub fn register(&mut self, name: char) -> Option<&std::sync::Arc<str>> {
    self.reader.registers.get(&name)
  }

  pub fn take_yank(&mut self) -> Option<Yank> {
    self.reader.pending_yank.take()
  }

  pub fn deliver_yank(&mut self, yank: Yank) {
    self.reader.pending_yank = Some(yank);
  }

  pub fn store(&mut self, register: char, text: String) -> Result<()> {
    self.reader.store(register, text)
  }

  pub fn take_operator(&mut self) -> Option<Operator> {
    self.reader.operator.take()
  }

  pub fn multiply_operator(&mut self, count: usize) {
    if let Some(operator) = &mut self.reader.operator {
      operator.count = operator.count.saturating_mul(count);
    }
  }

  pub fn cancel(&mut self) {
    self.reader.cancel();
  }

  pub fn disarm(&mut self) {
    self.reader.disarm();
  }

  pub fn normal(&mut self) {
    self.reader.normal();
  }

  pub fn enter_cursor(&mut self) {
    self.reader.enter_cursor();
  }

  pub fn visual(&mut self, kind: SelectionKind) {
    self.reader.visual(kind);
  }

  pub fn end_selection(&mut self) {
    self.reader.end_selection();
  }

  pub fn clear_history(&mut self) {
    self.reader.clear_history();
  }

  pub fn describe_jump(&mut self, description: Option<JumpDescription>) {
    self.reader.describe_jump(description);
  }

  pub fn finish_search_jumps(&mut self, query: &Query, hits: &[Hit]) {
    self.reader.finish_search_jumps(query, hits);
  }

  pub fn record(&mut self, origin: Location) {
    self.reader.record(origin);
  }
}
