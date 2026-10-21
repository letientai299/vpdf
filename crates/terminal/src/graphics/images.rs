use super::Tracking;
use super::{
  BLOCK, Cache, Graphics, HashMap, Key, LOST, OVERLAYS, Ordering, PREVIEW,
  Payload, Result, STORED, Spool, TILE_END, Tile, Usage, Write, transmit,
};

impl Graphics {
  /// Whether overlays need exact-size images.
  pub fn crop_quirk(&self) -> bool {
    self.quirk
  }

  pub fn contains(&mut self, key: &Key) -> bool {
    self.resident.get(key).is_some()
  }

  /// Whether `key` is resident or kept for upload.
  pub fn has(&mut self, key: &Key) -> bool {
    self.resident.get(key).is_some() || self.stored.get(key).is_some()
  }

  /// Deletes a pane's tiles at the next frame.
  pub fn forget(&mut self, pane: u32) {
    let keys: Vec<_> =
      self.ids.keys().filter(|key| key.0 == pane).copied().collect();
    for key in keys {
      self.resident.remove(&key);
      self.stale.extend(self.ids.remove(&key));
    }
    let kept: Vec<_> = self
      .stored
      .entries()
      .map(|(key, _)| *key)
      .filter(|k| k.0 == pane)
      .collect();
    for key in kept {
      self.stored.remove(&key);
    }
  }

  /// Forgets terminal images after another program had the screen; kept
  /// tiles upload again when next placed.
  pub fn lost(&mut self) {
    let stored = std::mem::replace(&mut self.stored, Cache::new(0));
    let uploaded = self.uploaded;
    *self = Self {
      stored,
      uploaded,
      transport: self.transport,
      budget: self.budget,
      tracking: self.tracking,
      base: self.base,
      next: self.base + OVERLAYS,
      overlay_end: self.overlay_end,
      quirk: self.quirk,
      cell_offsets: self.cell_offsets,
      quiet: self.quiet,
      resident: Cache::new(self.budget),
      ..Self::default()
    };
  }

  /// Handles the terminal's `ENOENT` for image `id`: it evicted the
  /// image, so it uploads again. A late reply can name an id uploaded
  /// again since, so the next frame deletes the id before its reuse.
  /// Returns whether `id` was one in use.
  pub fn missing(&mut self, id: u32) -> bool {
    self.unplace(id);
    if id == self.base + PREVIEW {
      self.preview = LOST;
      return true;
    }
    if let Some(solid) = find(&self.solids, id) {
      self.solids.remove(&solid);
      self.retired.push(id);
      return true;
    }
    let Some(key) = find(&self.ids, id) else { return false };
    self.ids.remove(&key);
    self.resident.remove(&key);
    self.stale.push(id);
    true
  }

  /// Forgets placements of `id`, whose image is gone.
  pub(super) fn unplace(&mut self, id: u32) {
    self.shown.retain(|pair| pair.0 != id);
    self.placing.retain(|pair| pair.0 != id);
  }

  /// Keeps pane `pane`'s tile until a frame places it.
  pub fn keep(&mut self, pane: u32, tile: Tile) {
    let weight = match &tile.payload {
      Payload::Encoded(encoded) => encoded.len(),
      // The spool holds the pixels; this keeps about one entry per slot.
      Payload::Spooled { slot, .. } => slot.len * STORED / crate::spool::BYTES,
    };
    self.stored.insert((pane, tile.key), tile.payload, weight);
  }

  pub fn upload(
    &mut self,
    pane: u32,
    tile: &Tile,
    out: &mut impl Write,
  ) -> Result<()> {
    let key = (pane, tile.key);
    if !self.contains(&key) {
      self.send(key, &tile.payload, out)?;
    }
    Ok(())
  }

  /// Transmits a tile and returns its image id.
  pub(super) fn send(
    &mut self,
    key: Key,
    payload: &Payload,
    out: &mut impl Write,
  ) -> Result<u32> {
    let id = self.free.pop().unwrap_or_else(|| {
      self.next += 1;
      if self.tracking == Tracking::Process {
        TILE_END.fetch_max(self.next, Ordering::Relaxed);
      }
      self.next - 1
    });
    let tile = key.1;
    let bytes = match payload {
      Payload::Encoded(encoded) => {
        transmit(
          self.transport,
          out,
          id,
          tile.width,
          tile.height,
          24,
          encoded,
        )?;
        encoded.len()
      }
      Payload::Spooled { spool, slot } => {
        let control = format!(
          "a=t,f=24,i={id},q=2,s={},v={},{}",
          tile.width,
          tile.height,
          Spool::keys(*slot)
        );
        self.transport.graphics(out, &control, &spool.payload)?;
        slot.len
      }
    };
    // Weight floor keeps residency within tile ids.
    let weight = ((tile.width * tile.height * 3) as usize)
      .max(self.budget.div_ceil((BLOCK - OVERLAYS) as usize));
    for key in self.resident.insert(key, id, weight) {
      if let Some(id) = self.ids.remove(&key) {
        self.transport.graphics(out, &format!("a=d,d=I,i={id},q=2"), "")?;
        self.unplace(id);
        self.free.push(id);
      }
    }
    self.ids.insert(key, id);
    self.uploaded += bytes as u64;
    Ok(id)
  }

  pub fn owner_bytes(&self, owner: u32) -> usize {
    self
      .resident
      .entries()
      .filter(|(key, _)| key.0 == owner)
      .map(|(key, _)| self.resident.weight(key).unwrap_or(0))
      .sum::<usize>()
      + self
        .stored
        .entries()
        .filter(|(key, _)| key.0 == owner)
        .map(|(key, _)| self.stored.weight(key).unwrap_or(0))
        .sum::<usize>()
  }

  pub fn usage(&self) -> Usage {
    Usage { bytes: self.bytes(), uploaded: self.uploaded }
  }

  pub fn bytes(&self) -> usize {
    self.resident.used()
  }
}

/// The key of `map` whose value is `id`.
fn find<K: Copy>(map: &HashMap<K, u32>, id: u32) -> Option<K> {
  map.iter().find_map(|(key, value)| (*value == id).then_some(*key))
}
