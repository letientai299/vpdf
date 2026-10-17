use crate::Result;
use crate::app::App;
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

  pub fn scroll(&mut self, delta: f32) -> Result<()> {
    if self.text_mode {
      let rows = (delta / f32::from(self.screen.cell_height)).round();
      self.scroll_text(rows as isize)?;
      return self.ensure_cursor();
    }
    let start =
      self.animation.as_ref().map_or(self.reader.scroll, |s| s.target);
    let target = self.layout.clamp_scroll(start + delta);
    self.animation = Some(Scroll { target, updated: Instant::now() });
    let cursor = self.reader.cursor;
    if let Some(glyph) = self.doc.text(cursor.page)?.glyph(cursor) {
      let rect = self.layout.project(cursor.page, glyph.bounds, target);
      if rect.y0 >= 0.0 && rect.y1 <= self.layout.screen.height() {
        return Ok(());
      }
    }
    let anchor = target + self.layout.screen.height() / 2.0;
    let page = self.layout.page_at(anchor);
    let text = self.doc.text(page)?;
    let layout = &self.layout.pages[page.0];
    if let Some(inverse) = layout.matrix.invert() {
      let (x, y) = inverse.transform_xy(self.layout.pan, anchor - layout.top);
      self.reader.cursor = text.nearest(page, x, y);
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
