use super::{Bounds, Graphics, Key, Result, Screen, Write};
use vpdf_model::geometry::{clip, union_area};

impl Graphics {
  pub(super) fn stand_in(
    &mut self,
    key: Key,
    origin: (f32, f32),
    screen: Screen,
    left: u16,
    out: &mut impl Write,
  ) -> Result<bool> {
    let tile = key.1;
    let target =
      Bounds::placed(origin, (tile.width as f32, tile.height as f32));
    let Some(target) = super::placement::clip(target, screen) else {
      return Ok(true);
    };
    if tile.scale == 0 {
      return Ok(false);
    }
    let page = (origin.0 - tile.x as f32, origin.1 - tile.y as f32);
    let mut candidates: Vec<_> = self
      .resident
      .entries()
      .map(|(key, _)| key)
      .chain(self.stored.entries().map(|(key, _)| key))
      .filter(|other| {
        other.0 == key.0
          && other.1.page == tile.page
          && other.1.rotation == tile.rotation
          && other.1.crop == tile.crop
          && other.1.scale != tile.scale
          && other.1.scale != 0
      })
      .filter_map(|other| {
        let other = other.1;
        let ratio = tile.scale as f32 / other.scale as f32;
        let rect = Bounds {
          x0: page.0 + other.x as f32 * ratio,
          y0: page.1 + other.y as f32 * ratio,
          x1: page.0 + (other.x + other.width) as f32 * ratio,
          y1: page.1 + (other.y + other.height) as f32 * ratio,
        };
        let shown = super::placement::clip(rect, screen)?;
        clip(cells(shown, screen), target).map(|_| (other, rect))
      })
      .collect();
    let distance =
      |scale: u32| (f64::from(scale) / f64::from(tile.scale)).ln().abs();
    candidates.sort_by(|a, b| {
      distance(a.0.scale)
        .total_cmp(&distance(b.0.scale))
        .then_with(|| (a.0.y, a.0.x).cmp(&(b.0.y, b.0.x)))
    });
    candidates.dedup_by_key(|(key, _)| *key);
    let mut covered = Vec::new();
    for (other, rect) in candidates {
      let cached = (key.0, other);
      let id = match self.resident.get(&cached) {
        Some(&id) => id,
        None => {
          let Some(payload) = self.kept(cached) else { continue };
          self.send(cached, &payload, out)?
        }
      };
      let Some(shown) = super::placement::clip(rect, screen) else { continue };
      let destination = cells(shown, screen);
      let ratio = other.scale as f32 / tile.scale as f32;
      let source = Bounds {
        x0: ((shown.x0 - rect.x0) * ratio).floor().max(0.0),
        y0: ((shown.y0 - rect.y0) * ratio).floor().max(0.0),
        x1: ((shown.x1 - rect.x0) * ratio).ceil().min(other.width as f32),
        y1: ((shown.y1 - rect.y0) * ratio).ceil().min(other.height as f32),
      };
      let z = -3 - (distance(other.scale) * 10_000.0) as i32;
      let y =
        if self.quirk { other.height as f32 - source.y1 } else { source.y0 };
      let control = format_args!(
        "z={z},x={},y={},w={},h={},c={},r={}",
        source.x0 as u32,
        y as u32,
        source.width() as u32,
        source.height() as u32,
        destination.width() as u32 / u32::from(screen.cell_width),
        destination.height() as u32 / u32::from(screen.cell_height),
      );
      // Reuse one placement across pending tiles.
      self.put(out, (id, 2), control, destination, left, screen)?;
      if let Some(area) = clip(destination, target) {
        covered.push(area);
      }
      if union_area(&covered) + 0.01 >= target.width() * target.height() {
        self.stand_ins += 1;
        return Ok(true);
      }
    }
    self.stand_ins += usize::from(!covered.is_empty());
    Ok(false)
  }
}

// Scaled placements need whole-cell destination bounds.
fn cells(rect: Bounds, screen: Screen) -> Bounds {
  let x0 = (rect.x0 / f32::from(screen.cell_width)).floor()
    * f32::from(screen.cell_width);
  let y0 = (rect.y0 / f32::from(screen.cell_height)).floor()
    * f32::from(screen.cell_height);
  let x1 = (rect.x1 / f32::from(screen.cell_width)).ceil()
    * f32::from(screen.cell_width);
  let y1 = (rect.y1 / f32::from(screen.cell_height)).ceil()
    * f32::from(screen.cell_height);
  Bounds {
    x0: x0.min(screen.width() - f32::from(screen.cell_width)),
    y0: y0.min(screen.height() - f32::from(screen.cell_height)),
    x1: x1.max(x0 + f32::from(screen.cell_width)).min(screen.width()),
    y1: y1.max(y0 + f32::from(screen.cell_height)).min(screen.height()),
  }
}
