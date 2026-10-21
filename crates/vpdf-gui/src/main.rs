use clap::Parser;
use eframe::egui::{self, FontId, ViewportCommand};
use std::path::PathBuf;
use vpdf::session::native::{Bridge, Frame, Input};
#[cfg(feature = "bench-events")]
use vpdf_gui::inject;
use vpdf_gui::{
  input,
  paint::{self, Renderer},
  trace::Trace,
};
use vpdf_model::geometry::Screen;

#[derive(Parser)]
#[command(version = vpdf::version::BUILD, about = "Native Vpdf reader")]
struct Args {
  files: Vec<PathBuf>,
  /// Launch page overriding the saved position.
  #[arg(long)]
  page: Option<usize>,
  /// Open without activating the window.
  #[arg(long)]
  background: bool,
  #[arg(long, hide = true)]
  capture: Option<PathBuf>,
  #[cfg(feature = "bench-events")]
  #[arg(long, hide = true)]
  events: Option<PathBuf>,
}

struct Window {
  bridge: Bridge,
  frame: Option<Frame>,
  renderer: Renderer,
  input: input::Adapter,
  screen: Option<Screen>,
  capture: Option<PathBuf>,
  #[cfg(feature = "bench-events")]
  events: Option<inject::Injector>,
  #[cfg(feature = "bench-events")]
  mark: Option<String>,
  trace: Trace,
}

impl Window {
  fn new(
    cc: &eframe::CreationContext<'_>,
    args: Args,
  ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
    vpdf_gui::fonts::install(&cc.egui_ctx);
    #[cfg(feature = "bench-events")]
    let events = args
      .events
      .as_deref()
      .map(|path| inject::spawn(path, cc.egui_ctx.clone()));
    let trace = Trace::from_env();
    let ctx = cc.egui_ctx.clone();
    let bridge =
      Bridge::start(args.files, args.page, move || ctx.request_repaint())?;
    if trace.enabled() {
      bridge.send(Input::Observe(true));
    }
    Ok(Self {
      bridge,
      frame: None,
      renderer: Renderer::default(),
      input: input::Adapter::default(),
      screen: None,
      capture: args.capture,
      #[cfg(feature = "bench-events")]
      events,
      #[cfg(feature = "bench-events")]
      mark: None,
      trace,
    })
  }

  fn send_input(&mut self, input: Input) {
    self.trace.input();
    self.bridge.send(input);
  }

  #[cfg(feature = "bench-events")]
  fn record_mark(&mut self) {
    if let Some(name) = self.mark.take()
      && let Some(trace) = self.trace.log()
    {
      self.bridge.send(Input::Mark { name, trace, ack: None });
    }
  }
}

impl eframe::App for Window {
  fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
    let started = std::time::Instant::now();
    let ctx = ui.ctx().clone();
    let scale = ctx.pixels_per_point();
    let font = FontId::monospace(paint::FONT_SIZE);
    let cell = ctx.fonts_mut(|fonts| {
      let width = fonts
        .layout_no_wrap("m".into(), font.clone(), egui::Color32::WHITE)
        .size()
        .x;
      egui::vec2(width, fonts.row_height(&font))
    });
    let physical = ui.max_rect().size() * scale;
    let cell_width = (cell.x * scale).ceil().max(1.0) as u16;
    let cell_height = (cell.y * scale).ceil().max(1.0) as u16;
    let screen = Screen {
      columns: (physical.x / f32::from(cell_width)).floor().max(1.0) as u16,
      rows: (physical.y / f32::from(cell_height)).floor().max(2.0) as u16,
      cell_width,
      cell_height,
      origin: (0, 0),
    };
    if self.screen != Some(screen) {
      self.screen = Some(screen);
      self.bridge.send(Input::Resize(screen));
    }
    let mut content = false;
    let mut received = false;
    if let Some(frame) = self.bridge.take() {
      content = frame.content;
      received = true;
      ctx.send_viewport_cmd(ViewportCommand::Title(frame.title.clone()));
      if frame.quit {
        ctx.send_viewport_cmd(ViewportCommand::Close);
      }
      if let Some(text) = &frame.clipboard {
        ctx.copy_text(text.clone());
      }
      self.frame = Some(frame);
    }
    self.input.editing(
      self.frame.as_ref().and_then(|frame| input::purpose(&frame.scene)),
    );
    let drawn = self.frame.as_ref().map_or(screen, |frame| frame.screen);
    let step = self.frame.as_ref().map_or([0.0; 2], Frame::wheel_step);
    let events = ctx.input(|input| {
      self.input.events(
        &input.events,
        input.modifiers,
        drawn,
        scale,
        step,
        input.pointer.latest_pos(),
      )
    });
    for event in events {
      self.send_input(event);
    }
    let drops = ctx.input(|input| input.raw.dropped_files.clone());
    for file in drops {
      self.send_input(Input::Open(file.path().to_path_buf()));
    }
    #[cfg(feature = "bench-events")]
    self.record_mark();
    if let Some(frame) = &self.frame {
      ctx.set_cursor_icon(input::cursor(frame.cursor));
      let before = self.renderer.upload_bytes;
      let mut ime = self.renderer.paint(
        ui.painter(),
        frame,
        frame.screen,
        scale,
        self.input.preedit(),
      );
      let interrupt = self.input.interrupt();
      if let Some(ime) = &mut ime {
        ime.should_interrupt_composition = interrupt;
      }
      ctx.output_mut(|output| output.ime = ime);
      let bytes = self.renderer.upload_bytes.saturating_sub(before);
      if received || bytes != 0 {
        self.bridge.send(Input::Presented { content, bytes });
      }
      self.trace.paint(frame, &self.renderer, started, received, bytes);
      if shows_images(frame)
        && let Some(path) = self.capture.take()
      {
        ctx.request_screenshot(move |image| capture(&image, &path));
      }
    }
    // Repaints of this frame keep the textures reloaded once.
    if let Some(frame) = &mut self.frame {
      frame.reloaded.clear();
    }
  }
  #[cfg(feature = "bench-events")]
  fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
    if let Some(events) = &mut self.events {
      self.mark = events.apply(ctx, raw);
    }
  }

  fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
    paint::background().to_normalized_gamma_f32()
  }
  fn on_exit(&mut self) {
    if let Err(error) = self.bridge.close() {
      eprintln!("Close failed: {error}");
    }
  }
}

/// A complete frame with pixels, worth a `--capture`.
fn shows_images(frame: &Frame) -> bool {
  frame.complete
    && (!frame.images.is_empty()
      || frame
        .scene
        .iter()
        .any(|op| matches!(op, vpdf_model::scene::Op::Picture(_))))
}

fn capture(image: &egui::ColorImage, path: &std::path::Path) {
  let bytes: Vec<_> =
    image.pixels.iter().flat_map(egui::Color32::to_array).collect();
  let Some(pixels) = image::RgbaImage::from_raw(
    image.size[0] as u32,
    image.size[1] as u32,
    bytes,
  ) else {
    return;
  };
  if let Err(error) = pixels.save(path) {
    eprintln!("Capture failed: {error}");
  }
}

// Winit activates on launch, ignoring `open -g`.
#[cfg(target_os = "macos")]
fn background(builder: &mut eframe::EventLoopBuilder<eframe::UserEvent>) {
  use winit::platform::macos::EventLoopBuilderExtMacOS;
  builder.with_activate_ignoring_other_apps(false);
}

#[cfg(not(target_os = "macos"))]
fn background(_: &mut eframe::EventLoopBuilder<eframe::UserEvent>) {}

fn icon() -> Result<egui::IconData, image::ImageError> {
  eframe::icon_data::from_png_bytes(include_bytes!(
    "../../../docs/assets/vpdf.png"
  ))
}

fn options(args: &Args) -> Result<eframe::NativeOptions, image::ImageError> {
  let mut options = eframe::NativeOptions {
    viewport: egui::ViewportBuilder::default()
      .with_title("Vpdf")
      .with_icon(icon()?)
      .with_inner_size([1100.0, 800.0])
      .with_active(!args.background),
    ..Default::default()
  };
  if args.background {
    options.event_loop_builder = Some(Box::new(background));
  }
  Ok(options)
}

fn main() -> eframe::Result {
  let args = vpdf::version::parse::<Args>(false);
  let options = options(&args)
    .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
  eframe::run_native(
    "Vpdf",
    options,
    Box::new(move |cc| Ok(Box::new(Window::new(cc, args)?))),
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn background_is_explicit() {
    for (flags, active) in [
      (vec![], true),
      (vec!["--background"], false),
      #[cfg(feature = "bench-events")]
      (vec!["--events", "input.fifo"], true),
      #[cfg(feature = "bench-events")]
      (vec!["--background", "--events", "input.fifo"], false),
    ] {
      let args =
        Args::try_parse_from(std::iter::once("vpdf-gui").chain(flags)).unwrap();
      let options = options(&args).unwrap();
      assert_eq!(options.viewport.active, Some(active));
      assert_eq!(options.event_loop_builder.is_some(), !active);
    }
  }

  #[test]
  #[cfg(not(feature = "bench-events"))]
  fn production_rejects_events() {
    assert!(
      Args::try_parse_from(["vpdf-gui", "--events", "input.fifo"]).is_err()
    );
  }

  #[test]
  fn icon_preserves_gradient() -> Result<(), image::ImageError> {
    let icon = icon()?;
    assert_eq!((icon.width, icon.height), (256, 256));
    let pixel = |x: usize, y: usize| {
      let offset = (y * icon.width as usize + x) * 4;
      &icon.rgba[offset..offset + 4]
    };
    let top = pixel(60, 60);
    let bottom = pixel(60, 150);
    for color in [top, bottom] {
      assert!(color[0] > 180 && color[1] < 128 && color[2] < 128);
      assert_eq!(color[3], 255);
    }
    assert!(top[1] > bottom[1]);
    assert_eq!(pixel(0, 0)[3], 0);
    Ok(())
  }
}
