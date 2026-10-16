use crate::Result;
use crate::app::App;
use std::time::Instant;

pub struct Scroll {
  pub target: f32,
  pub updated: Instant,
}

impl App {
  pub fn animate(&mut self) -> bool {
    let Some(animation) = &mut self.animation else { return false };
    let now = Instant::now();
    let elapsed = now.duration_since(animation.updated).as_secs_f32();
    animation.updated = now;
    let distance = animation.target - self.reader.scroll;
    if distance.abs() < 0.5 {
      self.reader.scroll = animation.target;
      self.animation = None;
    } else {
      self.reader.scroll += distance * (1.0 - (-elapsed / 0.018).exp());
    }
    true
  }

  pub fn scroll(&mut self, delta: f32) -> Result<()> {
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
