use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

const SLOT_BYTES: u64 = 512 * 256 * 4;
const SLOTS: usize = 32;
const READY: u8 = 0;
const BUSY: u8 = 1;
const LOST: u8 = 2;

struct Slot {
  buffer: wgpu::Buffer,
  state: Arc<AtomicU8>,
}

/// egui's bind group keeps the texture alive until freed.
pub struct Handle {
  id: egui::TextureId,
  retired: Arc<Mutex<Vec<egui::TextureId>>>,
}

impl Handle {
  pub fn id(&self) -> egui::TextureId {
    self.id
  }
}

impl Drop for Handle {
  fn drop(&mut self) {
    self.retired.lock().unwrap().push(self.id);
  }
}

/// Copy rows must align to 256 bytes.
fn pitch(width: usize) -> usize {
  (width * 4).div_ceil(256) * 256
}

pub fn fits(size: [usize; 2]) -> bool {
  size[0] > 0
    && size[1] > 0
    && pitch(size[0])
      .checked_mul(size[1])
      .is_some_and(|bytes| bytes as u64 <= SLOT_BYTES)
}

pub struct Uploads {
  state: egui_wgpu::RenderState,
  slots: Vec<Slot>,
  commands: Option<wgpu::CommandEncoder>,
  retired: Arc<Mutex<Vec<egui::TextureId>>>,
}

impl Uploads {
  pub fn capacity(&self) -> usize {
    self.slots.len() * SLOT_BYTES as usize
  }
  pub fn new(state: egui_wgpu::RenderState) -> Self {
    Self { state, slots: Vec::new(), commands: None, retired: Arc::default() }
  }

  /// Frees ids retired before the previous frame rendered.
  pub fn begin(&mut self) {
    let _ = self.state.device.poll(wgpu::PollType::Poll);
    // Failed remaps would otherwise shrink the pool forever.
    self.slots.retain(|slot| slot.state.load(Ordering::Acquire) != LOST);
    let mut renderer = self.state.renderer.write();
    for id in self.retired.lock().unwrap().drain(..) {
      renderer.free_texture(&id);
    }
  }

  pub fn upload(&mut self, pixels: &[u8], size: [usize; 2]) -> Option<Handle> {
    assert!(fits(size), "upload exceeds staging slot");
    debug_assert_eq!(pixels.len(), size[0] * size[1] * 3, "tile RGB length");
    let pitch = pitch(size[0]);
    let index = self
      .slots
      .iter()
      .position(|slot| slot.state.load(Ordering::Acquire) == READY)
      .or_else(|| {
        if self.slots.len() == SLOTS {
          return None;
        }
        self.slots.push(Slot {
          buffer: self.state.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PDF upload slot"),
            size: SLOT_BYTES,
            usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: true,
          }),
          state: Arc::new(AtomicU8::new(READY)),
        });
        Some(self.slots.len() - 1)
      })?;
    let slot = &self.slots[index];
    slot.state.store(BUSY, Ordering::Release);
    {
      let mut target =
        slot.buffer.get_mapped_range_mut(..).expect("Mapped upload slot");
      rgba(target.slice(..), pixels, size[0], pitch);
    }
    slot.buffer.unmap();
    let texture = self.state.device.create_texture(&wgpu::TextureDescriptor {
      label: Some("PDF tile"),
      size: wgpu::Extent3d {
        width: size[0] as u32,
        height: size[1] as u32,
        depth_or_array_layers: 1,
      },
      mip_level_count: 1,
      sample_count: 1,
      dimension: wgpu::TextureDimension::D2,
      format: wgpu::TextureFormat::Rgba8Unorm,
      usage: wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::COPY_DST,
      view_formats: &[],
    });
    let id = self.state.renderer.write().register_native_texture(
      &self.state.device,
      &texture.create_view(&wgpu::TextureViewDescriptor::default()),
      wgpu::FilterMode::Linear,
    );
    let encoder = self.commands.get_or_insert_with(|| {
      self
        .state
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default())
    });
    encoder.copy_buffer_to_texture(
      wgpu::TexelCopyBufferInfo {
        buffer: &slot.buffer,
        layout: wgpu::TexelCopyBufferLayout {
          offset: 0,
          bytes_per_row: Some(pitch as u32),
          rows_per_image: Some(size[1] as u32),
        },
      },
      texture.as_image_copy(),
      texture.size(),
    );
    let state = Arc::clone(&slot.state);
    encoder.map_buffer_on_submit(
      &slot.buffer,
      wgpu::MapMode::Write,
      ..,
      move |result| {
        state
          .store(if result.is_ok() { READY } else { LOST }, Ordering::Release);
      },
    );
    Some(Handle { id, retired: Arc::clone(&self.retired) })
  }

  /// Submits copies ahead of egui, which drops skipped frames.
  pub fn finish(&mut self) {
    if let Some(commands) = self.commands.take() {
      self.state.queue.submit([commands.finish()]);
    }
  }
}

fn rgba(
  mut target: wgpu::WriteOnly<'_, [u8]>,
  pixels: &[u8],
  width: usize,
  pitch: usize,
) {
  for (row, rgb) in pixels.chunks_exact(width * 3).enumerate() {
    target
      .slice(row * pitch..row * pitch + width * 4)
      .into_chunks::<4>()
      .0
      .write_iter(
        rgb.as_chunks::<3>().0.iter().map(|rgb| [rgb[0], rgb[1], rgb[2], 255]),
      );
  }
}

impl Drop for Uploads {
  fn drop(&mut self) {
    self.finish();
    self.begin();
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn padded_rows_preserve_rgb() {
    let mut target = [17; 512];
    rgba(wgpu::WriteOnly::from_mut(&mut target), &[1, 2, 3, 4, 5, 6], 1, 256);
    assert_eq!(&target[..4], &[1, 2, 3, 255]);
    assert_eq!(&target[256..260], &[4, 5, 6, 255]);
    assert!(target[4..256].iter().all(|byte| *byte == 17));
  }

  #[cfg(target_os = "macos")]
  #[test]
  #[ignore = "requires native GPU"]
  fn slots_wait_for_submission() {
    let instance = wgpu::Instance::default();
    let state = pollster::block_on(egui_wgpu::RenderState::create(
      &egui_wgpu::WgpuConfiguration::default(),
      &instance,
      None,
      egui_wgpu::RendererOptions::default(),
    ))
    .unwrap();
    let mut uploads = Uploads::new(state.clone());
    let handles: Vec<_> =
      (0..SLOTS).map(|_| uploads.upload(&[1, 2, 3], [1, 1]).unwrap()).collect();
    assert!(uploads.upload(&[1, 2, 3], [1, 1]).is_none());
    assert_eq!(uploads.capacity(), SLOTS * SLOT_BYTES as usize);
    uploads.finish();
    state
      .device
      .poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(5)),
      })
      .unwrap();
    assert!(
      uploads
        .slots
        .iter()
        .all(|slot| slot.state.load(Ordering::Acquire) == READY)
    );
    let ids: Vec<_> = handles.iter().map(Handle::id).collect();
    drop(handles);
    assert_eq!(uploads.retired.lock().unwrap().len(), SLOTS);
    uploads.begin();
    assert!(uploads.retired.lock().unwrap().is_empty());
    assert!(ids.iter().all(|id| state.renderer.read().texture(id).is_none()));
  }
}
