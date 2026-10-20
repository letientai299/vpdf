use clap::Parser;
use eframe::egui::{self, FontId, ViewportCommand};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use vpdf::session::native::{Bridge, Frame, Input};
use vpdf_gui::{
  inject, input,
  paint::{self, Renderer},
};
use vpdf_model::geometry::Screen;

#[derive(Parser)]
#[command(version, about = "Native vpdf reader")]
struct Args {
  files: Vec<PathBuf>,
  #[arg(long, hide = true)]
  capture: Option<PathBuf>,
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
  events: Option<Receiver<inject::Injected>>,
}

impl Window {
  fn new(
    cc: &eframe::CreationContext<'_>,
    args: Args,
  ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
    vpdf_gui::fonts::install(&cc.egui_ctx);
    let events = args
      .events
      .as_deref()
      .map(|path| inject::spawn(path, cc.egui_ctx.clone()));
    let ctx = cc.egui_ctx.clone();
    let bridge = Bridge::start(args.files, move || ctx.request_repaint())?;
    Ok(Self {
      bridge,
      frame: None,
      renderer: Renderer::default(),
      input: input::Adapter::default(),
      screen: None,
      capture: args.capture,
      events,
    })
  }
}

impl eframe::App for Window {
  fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
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
      self.bridge.send(event);
    }
    let drops = ctx.input(|input| input.raw.dropped_files.clone());
    for file in drops {
      self.bridge.send(Input::Open(file.path().to_path_buf()));
    }
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
      if frame.complete
        && (!frame.images.is_empty()
          || frame
            .scene
            .iter()
            .any(|op| matches!(op, vpdf_model::scene::Op::Picture(_))))
        && let Some(path) = self.capture.take()
      {
        ctx.request_screenshot(move |image| capture(&image, &path));
      }
    }
  }
  fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
    if let Some(events) = &self.events {
      inject::apply(events, ctx, raw);
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

fn main() -> eframe::Result {
  let args = Args::parse();
  let options = eframe::NativeOptions {
    viewport: egui::ViewportBuilder::default()
      .with_title("vpdf")
      .with_inner_size([1100.0, 800.0])
      .with_active(false),
    event_loop_builder: Some(Box::new(background)),
    ..Default::default()
  };
  eframe::run_native(
    "vpdf",
    options,
    Box::new(move |cc| Ok(Box::new(Window::new(cc, args)?))),
  )
}
