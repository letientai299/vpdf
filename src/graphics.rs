use crate::Result;
use crate::cache::Cache;
use crate::document::MIB;
use crate::layout::{Layout, Screen};
use crate::terminal::graphics;
use crate::text::Bounds;
use crate::tiles::{Tile, TileKey, encode};
use std::collections::HashMap;
use std::io::Write;

pub struct Graphics {
  resident: Cache<TileKey, u32>,
  ids: HashMap<TileKey, u32>,
  next: u32,
  overlay: u32,
  pub uploaded: u64,
}

impl Default for Graphics {
  fn default() -> Self {
    Self {
      resident: Cache::new(128 * MIB),
      ids: HashMap::new(),
      next: 1000,
      overlay: 1,
      uploaded: 0,
    }
  }
}

impl Graphics {
  pub fn contains(&mut self, key: &TileKey) -> bool {
    self.resident.get(key).is_some()
  }

  pub fn upload(&mut self, tile: &Tile, out: &mut impl Write) -> Result<()> {
    if self.contains(&tile.key) {
      return Ok(());
    }
    let id = self.next;
    self.next = self.next.checked_add(1).unwrap_or(1000);
    transmit(out, id, tile.key.width, tile.key.height, 24, &tile.encoded)?;
    let weight = (tile.key.width * tile.key.height * 3) as usize;
    for key in self.resident.insert(tile.key, id, weight) {
      if let Some(id) = self.ids.remove(&key) {
        graphics(out, &format!("a=d,d=I,i={id},q=2"), "")?;
      }
    }
    self.ids.insert(tile.key, id);
    self.uploaded += tile.encoded.len() as u64;
    Ok(())
  }

  pub fn begin(&mut self, out: &mut impl Write) -> Result<()> {
    graphics(out, "a=d,d=a,q=2", "")?;
    for id in 1..self.overlay {
      graphics(out, &format!("a=d,d=I,i={id},q=2"), "")?;
    }
    self.overlay = 1;
    Ok(())
  }

  pub fn place(
    &mut self,
    key: &TileKey,
    layout: &Layout,
    scroll: f32,
    left: u16,
    out: &mut impl Write,
  ) -> Result<bool> {
    let Some(&id) = self.resident.get(key) else { return Ok(false) };
    let x = key.x as f32 - layout.pan;
    let y = layout.pages[key.page.0].top + key.y as f32 - scroll;
    let rect = Bounds {
      x0: x,
      y0: y,
      x1: x + key.width as f32,
      y1: y + key.height as f32,
    };
    let Some(clip) = clip(rect, layout.screen) else { return Ok(true) };
    placement(
      out,
      &(format!(
        "a=p,i={id},p=1,q=2,C=1,z=-1,x={},y={},w={},h={}",
        (clip.x0 - x) as u32,
        (clip.y0 - y) as u32,
        clip.width() as u32,
        clip.height() as u32,
      ) + &offsets(clip, layout.screen)),
      clip,
      left,
      layout.screen,
    )?;
    Ok(true)
  }

  pub fn highlight(
    &mut self,
    rect: Bounds,
    color: [u8; 4],
    screen: Screen,
    left: u16,
    out: &mut impl Write,
  ) -> Result<()> {
    let Some(rect) = clip(rect, screen) else { return Ok(()) };
    if self.overlay >= 999 {
      return Ok(());
    }
    let width = rect.width().ceil().max(1.0) as i32;
    let height = rect.height().ceil().max(1.0) as i32;
    let pixels = color.repeat((width * height) as usize);
    let id = self.overlay;
    self.overlay += 1;
    transmit(out, id, width, height, 32, &encode(&pixels)?)?;
    placement(
      out,
      &(format!("a=p,i={id},q=2,C=1,z=1") + &offsets(rect, screen)),
      rect,
      left,
      screen,
    )?;
    Ok(())
  }

  pub fn bytes(&self) -> usize {
    self.resident.used()
  }
}

fn clip(rect: Bounds, screen: Screen) -> Option<Bounds> {
  let clipped = Bounds {
    x0: rect.x0.max(0.0).floor(),
    y0: rect.y0.max(0.0).floor(),
    x1: rect.x1.min(screen.width()).ceil(),
    y1: rect.y1.min(screen.height()).ceil(),
  };
  (clipped.width() > 0.0 && clipped.height() > 0.0).then_some(clipped)
}

fn offsets(rect: Bounds, screen: Screen) -> String {
  format!(
    ",X={},Y={}",
    rect.x0 as u32 % u32::from(screen.cell_width),
    rect.y0 as u32 % u32::from(screen.cell_height)
  )
}

fn placement(
  out: &mut impl Write,
  control: &str,
  rect: Bounds,
  left: u16,
  screen: Screen,
) -> Result<()> {
  let col = rect.x0 as u32 / u32::from(screen.cell_width) + u32::from(left) + 1;
  let row = rect.y0 as u32 / u32::from(screen.cell_height) + 1;
  crate::terminal::placement(
    out,
    control,
    col + u32::from(screen.origin.0),
    row + u32::from(screen.origin.1),
  )
}

fn transmit(
  out: &mut impl Write,
  id: u32,
  width: i32,
  height: i32,
  format: u8,
  encoded: &str,
) -> Result<()> {
  let mut chunks = encoded.as_bytes().chunks(4096).peekable();
  let mut first = true;
  while let Some(chunk) = chunks.next() {
    let more = u8::from(chunks.peek().is_some());
    let control = if first {
      format!("a=t,f={format},o=z,i={id},q=2,s={width},v={height},m={more}")
    } else {
      format!("m={more},q=2")
    };
    graphics(out, &control, std::str::from_utf8(chunk)?)?;
    first = false;
  }
  Ok(())
}
