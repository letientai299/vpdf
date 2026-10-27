use super::{Frame, View};
use crate::Result;
use crate::layout::Layout;
use crate::page::scroll::Scroll;
use crate::text::Position;
use crate::text_view::TextView;
use std::ops::Deref;
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
  pub fn select_document(&mut self, selected: bool) {
    if selected {
      self.reader.cancel();
    }
    self.reader.document_selected = selected;
  }

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

  pub fn settle_scroll(&mut self) -> bool {
    let Some(target) = self.finish_scroll() else { return false };
    self.reader.scroll = target;
    true
  }

  pub fn animate_to(&mut self, target: f32, now: std::time::Instant) {
    self.tracking.animation.start(target, now);
  }

  pub fn landing(&self) -> &crate::page::landing::Landing {
    &self.tracking.landing
  }

  /// What a frame paints over the text: cursor, selection, matches,
  /// landing, and hovered link areas.
  pub fn marks<'a>(
    &'a self,
    search: &'a crate::find::Search,
    cursorline: bool,
    dim: bool,
    hovered: &'a [(crate::text::PageId, crate::text::Bounds)],
  ) -> crate::paint::Marks<'a> {
    crate::paint::Marks {
      reader: &self.reader,
      matches: if search.hidden() { &[] } else { search.hits() },
      flash: self.tracking.landing.glyph(&self.reader),
      landing: self.tracking.landing.line(),
      cursorline,
      dim,
      hovered,
    }
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

  pub fn animate(&mut self, now: std::time::Instant) -> bool {
    let Some(scroll) = self.tracking.animation.step(self.reader.scroll, now)
    else {
      return false;
    };
    self.reader.scroll = scroll;
    true
  }

  /// Whether the action interrupts a pending operator.
  #[must_use]
  pub fn prepare(&mut self, action: crate::keys::Action) -> bool {
    use crate::keys::Action;
    self.tracking.animation.finish_before(action, &mut self.reader);
    self.reader.operator.is_some()
      && !matches!(
        action,
        Action::Motion(_)
          | Action::Object(..)
          | Action::Yank
          | Action::YankLine
          | Action::YankRaw
          | Action::YankPath
          | Action::Search(_)
          | Action::Cancel
      )
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

  pub fn cancel_content(&mut self) {
    if self.reader.mode() == vpdf_reader::reader::Mode::Cursor {
      self.reader.normal();
    }
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

  /// Forgets state tied to the replaced document: pending input,
  /// jumps, the scroll target, and the landing flash.
  pub fn reload(&mut self) {
    self.reader.cancel();
    self.reader.clear_history();
    self.tracking.target = None;
    self.tracking.landing.clear_flash();
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
