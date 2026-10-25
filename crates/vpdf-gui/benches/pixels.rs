use eframe::egui;
use eframe::egui_wgpu::{self, wgpu};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vpdf::session::native::{Frame, Image, Metrics};
use vpdf_gui::paint::Renderer;
use vpdf_model::geometry::Screen;
use vpdf_model::scene::{Op, Stage, TilePlacement};
use vpdf_model::tile::TileOwner;

#[path = "../../../benches/support/directory.rs"]
mod directory;
#[path = "../../../benches/support/hash.rs"]
mod hash;
#[path = "../../../benches/support/raster.rs"]
mod raster;

fn wait(state: &egui_wgpu::RenderState) {
  state
    .device
    .poll(wgpu::PollType::Wait {
      submission_index: None,
      timeout: Some(Duration::from_secs(5)),
    })
    .expect("GPU completion");
}

fn stats(mut values: Vec<f64>) -> serde_json::Value {
  values.sort_by(f64::total_cmp);
  let count = values.len();
  serde_json::json!({
    "n": count,
    "median": values[count / 2],
    "p95": values[95 * (count - 1) / 100],
    "p99": values[99 * (count - 1) / 100],
    "max": values[count - 1],
  })
}

fn paint(
  state: &egui_wgpu::RenderState,
  ctx: &egui::Context,
  renderer: &mut Renderer,
  frame: &Frame,
  view: &wgpu::TextureView,
  size: [u32; 2],
) -> (f64, f64, usize, usize) {
  let [width, height] = size;
  let (mut full, mut partial) = (0, 0);
  let before = Instant::now();
  let mut output = ctx.run_ui(
    egui::RawInput {
      screen_rect: Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(width as f32, height as f32),
      )),
      ..Default::default()
    },
    |ui| {
      renderer.paint(ui.painter(), frame, frame.screen, 1.0, None);
    },
  );
  let jobs = ctx.tessellate(output.shapes, output.pixels_per_point);
  let mut encoder = state
    .device
    .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
  let mut backend = state.renderer.write();
  for (id, deltas) in output.textures_delta.set.drain() {
    for delta in deltas {
      if delta.pos.is_some() {
        partial += 1;
      } else {
        full += 1;
      }
      backend.update_texture(&state.device, &state.queue, id, &delta);
    }
  }
  let extra = backend.update_buffers(
    &state.device,
    &state.queue,
    &mut encoder,
    &jobs,
    &egui_wgpu::ScreenDescriptor {
      size_in_pixels: [width, height],
      pixels_per_point: 1.0,
    },
  );
  {
    let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
      label: Some("pixel benchmark"),
      color_attachments: &[Some(wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
          load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
          store: wgpu::StoreOp::Store,
        },
      })],
      ..Default::default()
    });
    backend.render(
      &mut pass.forget_lifetime(),
      &jobs,
      &egui_wgpu::ScreenDescriptor {
        size_in_pixels: [width, height],
        pixels_per_point: 1.0,
      },
    );
  }
  state.queue.submit(extra.into_iter().chain([encoder.finish()]));
  for id in output.textures_delta.free.drain() {
    backend.free_texture(&id);
  }
  drop(backend);
  let submitted = Instant::now();
  wait(state);
  (
    (submitted - before).as_secs_f64() * 1e6,
    submitted.elapsed().as_secs_f64() * 1e6,
    full,
    partial,
  )
}

fn readback(
  state: &egui_wgpu::RenderState,
  target: &wgpu::Texture,
) -> vpdf::Result<Vec<u8>> {
  let (width, height) = (target.width(), target.height());
  let pitch = (width * 4).div_ceil(256) * 256;
  let readback = state.device.create_buffer(&wgpu::BufferDescriptor {
    label: Some("pixel fidelity"),
    size: u64::from(pitch * height),
    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
    mapped_at_creation: false,
  });
  let mut encoder = state
    .device
    .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
  encoder.copy_texture_to_buffer(
    target.as_image_copy(),
    wgpu::TexelCopyBufferInfo {
      buffer: &readback,
      layout: wgpu::TexelCopyBufferLayout {
        offset: 0,
        bytes_per_row: Some(pitch),
        rows_per_image: Some(height),
      },
    },
    target.size(),
  );
  state.queue.submit([encoder.finish()]);
  let (send, receive) = std::sync::mpsc::channel();
  readback.map_async(wgpu::MapMode::Read, .., move |result| {
    send.send(result).expect("map result");
  });
  wait(state);
  receive.recv()?.expect("mapped readback");
  let rgba: Vec<_> = readback
    .get_mapped_range(..)?
    .chunks(pitch as usize)
    .flat_map(|row| &row[..width as usize * 4])
    .copied()
    .collect();
  Ok(rgba)
}

fn main() -> vpdf::Result<()> {
  let (manifest, pixels) = raster::fixture()?;
  let instance = wgpu::Instance::default();
  let state = pollster::block_on(egui_wgpu::RenderState::create(
    &egui_wgpu::WgpuConfiguration::default(),
    &instance,
    None,
    egui_wgpu::RendererOptions::default(),
  ))?;
  let width = manifest.width as u32;
  let height = manifest.height as u32;
  let screen = Screen {
    columns: (width / 8) as u16,
    rows: (height / 16) as u16,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let target = state.device.create_texture(&wgpu::TextureDescriptor {
    label: Some("pixel benchmark"),
    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    mip_level_count: 1,
    sample_count: 1,
    dimension: wgpu::TextureDimension::D2,
    format: state.target_format,
    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
      | wgpu::TextureUsages::COPY_SRC,
    view_formats: &[],
  });
  let view = target.create_view(&wgpu::TextureViewDescriptor::default());
  let ctx = egui::Context::default();
  let mut renderer = Renderer::with_gpu(state.clone());
  let mut cpu = Vec::new();
  let mut waits = Vec::new();
  let mut owners = Vec::new();
  let mut full = 0;
  let mut partial = 0;
  let owner = TileOwner::new(1);
  let started = Instant::now();
  for index in 0..1201 {
    assert!(started.elapsed() < Duration::from_secs(30), "Benchmark deadline");
    let mut key = manifest.key();
    key.page = vpdf_model::text::PageId(index);
    let id = vpdf_model::tile::TileId { owner, key };
    let pixels = Arc::new(pixels.clone());
    owners.push(Arc::downgrade(&pixels));
    let frame = Frame {
      metrics: Metrics::default(),
      screen,
      scene: vec![Op::Tile(Box::new(TilePlacement::new(
        id,
        (0.0, 0.0),
        Stage { screen, left: 0 },
      )))],
      images: vec![Image { id, pixels }],
      owners: vec![owner],
      reloaded: Vec::new(),
      title: String::new(),
      cursor: vpdf_model::style::Shape::Default,
      complete: true,
      content: true,
      quit: false,
      clipboard: None,
    };
    let (submitted, waited, allocations, updates) =
      paint(&state, &ctx, &mut renderer, &frame, &view, [width, height]);
    if index >= 200 {
      cpu.push(submitted);
      waits.push(waited);
      full += allocations;
      partial += updates;
    }
    assert_eq!(renderer.coverage.missing, 0);
  }
  let rgba = readback(&state, &target)?;
  let retained: usize = owners
    .iter()
    .filter_map(std::sync::Weak::upgrade)
    .map(|pixels| pixels.capacity())
    .sum();
  println!(
    "{}",
    serde_json::json!({
      "kind": "pixels",
      "adapter": format!("{:?}", state.adapter.get_info()),
      "samples": 1001,
      "warmup_frames": 200,
      "cpu_submit_us": stats(cpu),
      "gpu_wait_us": stats(waits),
      "full_deltas": full,
      "partial_deltas": partial,
      "uploaded": renderer.uploaded,
      "upload_bytes": renderer.upload_bytes,
      "retained_rgb_bytes": retained,
      "output_sha256": hash::digest(&rgba),
      "fixture_sha256": manifest.rgb_sha256,
      "staging_capacity_bytes": renderer.staging_capacity(),
    })
  );
  Ok(())
}
