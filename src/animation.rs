use crate::Result;
use crate::app::App;
use crate::keys::Action;
use crate::reader::Mode;
use crate::text::{PageId, Position};
use std::time::Instant;

pub struct Scroll {
  pub target: f32,
  pub updated: Instant,
}

/// Next scroll toward `target` after `elapsed` seconds, or `None` once
/// it arrives.
#[expect(clippy::float_cmp, reason = "detects a step that rounds back")]
fn ease(scroll: f32, target: f32, elapsed: f32) -> Option<f32> {
  let distance = target - scroll;
  let next = scroll + distance * (1.0 - (-elapsed / 0.018).exp());
  // f32 spacing reaches 0.5 px past 4M px, where a short step rounds back
  // and the animation would never end.
  (distance.abs() >= 0.5 && next != scroll).then_some(next)
}

impl App {
  pub fn animate(&mut self) -> bool {
    let Some(animation) = &mut self.animation else { return false };
    let now = Instant::now();
    let elapsed = now.duration_since(animation.updated).as_secs_f32();
    animation.updated = now;
    match ease(self.reader.scroll, animation.target, elapsed) {
      Some(next) => self.reader.scroll = next,
      None => {
        self.reader.scroll = animation.target;
        self.animation = None;
      }
    }
    true
  }

  pub(crate) fn finish_scroll_before(&mut self, action: Action) {
    if self.reader.uses_cursor()
      && !matches!(action, Action::Scroll(_) | Action::Rows(_))
    {
      self.finish_scroll();
    }
  }

  fn finish_scroll(&mut self) {
    if let Some(animation) = self.animation.take() {
      self.reader.scroll = animation.target;
    }
  }

  pub fn scroll(&mut self, delta: f32) -> Result<()> {
    self.scroll_cursor(delta, false, self.reader.mode() == Mode::Cursor)
  }

  pub(crate) fn keyboard_scroll(
    &mut self,
    delta: f32,
    follow: bool,
  ) -> Result<()> {
    self.scroll_cursor(delta, follow, self.reader.uses_cursor())
  }

  fn cursor_point(&mut self) -> Result<(f32, f32)> {
    if self.text_mode {
      let (column, row) =
        self.text_cell(self.reader.cursor)?.unwrap_or((self.left, 0));
      return Ok((f32::from(column), f32::from(row)));
    }
    let cursor = self.reader.cursor;
    let text = self.doc.text(cursor.page)?;
    Ok(text.glyph(cursor).map_or((0.0, 0.0), |glyph| {
      self
        .layout
        .project(cursor.page, glyph.bounds, self.reader.scroll)
        .centre()
    }))
  }

  fn nearest_visible(&mut self, point: (f32, f32)) -> Result<Option<Position>> {
    if self.text_mode {
      return self.nearest_text(point);
    }
    let mut nearest = None;
    let mut distance = (u8::MAX, f32::INFINITY, f32::INFINITY);
    for page in self.layout.visible(self.reader.scroll) {
      let page = PageId(page);
      let text = self.doc.text(page)?;
      let glyphs = text.lines.iter().enumerate().flat_map(|(line, row)| {
        row
          .glyphs
          .iter()
          .enumerate()
          .map(move |(glyph, item)| (line, glyph, item))
      });
      for (line, glyph, item) in glyphs {
        if item.ch.is_whitespace() || !self.bounds_shown(page, item.bounds) {
          continue;
        }
        let rect = self.layout.project(page, item.bounds, self.reader.scroll);
        let (x, y) = rect.centre();
        let partial =
          u8::from(rect.y0 < 0.0 || rect.y1 > self.uncovered().height());
        let next = (partial, (y - point.1).abs(), (x - point.0).abs());
        if next < distance {
          distance = next;
          nearest = Some(Position { page, line, glyph });
        }
      }
    }
    Ok(nearest)
  }

  fn scroll_cursor(
    &mut self,
    delta: f32,
    follow: bool,
    sync: bool,
  ) -> Result<()> {
    self.search.yank = None;
    self.reader.disarm();
    let displayed = self.reader.scroll;
    self.finish_scroll();
    let mut point = if sync { self.cursor_point()? } else { (0.0, 0.0) };
    if self.text_mode {
      let rows = (delta / f32::from(self.screen.cell_height)).round();
      self.scroll_text(rows as isize)?;
      if !follow {
        point.1 -= rows;
      }
    } else {
      let start = self.reader.scroll;
      let target = self.layout.clamp_scroll(start + delta);
      self.reader.scroll = target;
      if !follow {
        point.1 -= target - start;
      }
      self.animation = Some(Scroll { target, updated: Instant::now() });
    }
    if sync
      && (follow || !self.visible_position(self.reader.cursor)?)
      && let Some(cursor) = self.nearest_visible(point)?
    {
      self.reader.cursor = cursor;
      self.reader.desired = None;
      self.target = Some(cursor);
    }
    if !self.text_mode {
      self.reader.scroll = displayed;
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn easing_arrives_far_down_long_documents() {
    assert!(ease(0.0, 100.0, 0.008).is_some_and(|next| next > 30.0));
    assert_eq!(ease(99.8, 100.0, 0.008), None);
    // 4.38M px: f32 steps by 0.5 px, so 0.18 px rounds back.
    assert_eq!(ease(4_384_724.5, 4_384_725.0, 0.008), None);
  }
}
