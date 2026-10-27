use clap::Parser;
use eframe::egui::{self, ViewportCommand};
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
  #[command(flatten)]
  config: vpdf::config::Options,
  /// PDF paths or file URIs, e.g. book.pdf#page=2.
  #[arg(value_name = "PDF")]
  files: Vec<PathBuf>,
  /// Open without activating the window.
  #[arg(long)]
  background: bool,
  #[arg(long, hide = true)]
  capture: Option<PathBuf>,
  #[cfg(feature = "bench-events")]
  #[arg(long, hide = true)]
  events: Option<PathBuf>,
}

fn system_palette(ctx: &egui::Context) -> vpdf_model::theme::Palette {
  match ctx.system_theme() {
    Some(egui::Theme::Light) => vpdf_model::theme::Palette::light(),
    Some(egui::Theme::Dark) | None => vpdf_model::theme::Palette::dark(),
  }
}

fn set_theme(ctx: &egui::Context, palette: vpdf_model::theme::Palette) {
  // Window overrides block system theme notifications.
  ctx.options_mut(|options| options.sync_window_theme = false);
  ctx.set_theme(if vpdf_model::theme::is_light(palette.background) {
    egui::Theme::Light
  } else {
    egui::Theme::Dark
  });
}

struct Window {
  #[cfg(target_os = "macos")]
  native: Option<vpdf_gui::platform::Native>,
  #[cfg(target_os = "macos")]
  documents: fmv_macos_events::Inbox,
  #[cfg(target_os = "macos")]
  preferences: vpdf_gui::preferences::Observer,
  bridge: Bridge,
  frame: Option<Frame>,
  renderer: Renderer,
  input: input::Adapter,
  screen: Option<Screen>,
  system_palette: vpdf_model::theme::Palette,
  palette: vpdf_model::theme::Palette,
  font_size: f32,
  capture: Option<PathBuf>,
  #[cfg(feature = "bench-events")]
  events: Option<inject::Injector>,
  #[cfg(feature = "bench-events")]
  mark: Option<String>,
  trace: Trace,
}

impl Window {
  fn refresh_system(&mut self, ctx: &egui::Context) {
    let system_palette = system_palette(ctx);
    if system_palette != self.system_palette {
      self.system_palette = system_palette;
      self.bridge.send(Input::SystemPalette(system_palette));
    }
  }

  fn new(
    cc: &eframe::CreationContext<'_>,
    args: Args,
    config: vpdf::config::Loaded,
    #[cfg(target_os = "macos")] documents: fmv_macos_events::Inbox,
  ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
    vpdf_gui::fonts::install(&cc.egui_ctx);
    #[cfg(feature = "bench-events")]
    let events = args
      .events
      .as_deref()
      .map(|path| inject::spawn(path, cc.egui_ctx.clone()));
    let font_size = config.settings.ui_font_size();
    let system_palette = system_palette(&cc.egui_ctx);
    let palette = config
      .settings
      .colors(vpdf_model::theme::Colors::Palette(system_palette))
      .palette();
    set_theme(&cc.egui_ctx, palette);
    let trace = Trace::from_env();
    let ctx = cc.egui_ctx.clone();
    let bridge = Bridge::configured(
      args
        .files
        .iter()
        .map(|file| vpdf_model::reference::Reference::parse(file.as_os_str()))
        .collect::<Result<Vec<_>, _>>()?,
      vpdf::state::location(),
      config,
      move || ctx.request_repaint(),
    )?;
    bridge.send(Input::SystemPalette(system_palette));
    if trace.enabled() {
      bridge.send(Input::Observe(true));
    }
    #[cfg(target_os = "macos")]
    let native = vpdf_gui::platform::Native::new(cc);
    // Minimized windows run no frames to drain the inbox.
    #[cfg(target_os = "macos")]
    {
      let ctx = cc.egui_ctx.clone();
      let restore = native.as_ref().map(vpdf_gui::platform::Native::restorer);
      documents.set_wake(move || {
        if let Some(restore) = &restore {
          restore();
        }
        ctx.request_repaint();
      });
    }
    let input = input::Adapter::default();
    #[cfg(target_os = "macos")]
    let input = {
      let mut input = input;
      if native.is_some() {
        for shortcut in vpdf_gui::platform::shortcuts() {
          input.reserve(shortcut);
        }
      }
      input
    };
    Ok(Self {
      #[cfg(target_os = "macos")]
      native,
      #[cfg(target_os = "macos")]
      documents,
      #[cfg(target_os = "macos")]
      preferences: {
        let ctx = cc.egui_ctx.clone();
        vpdf_gui::preferences::Observer::new(move || ctx.request_repaint())
      },
      bridge,
      frame: None,
      renderer: cc
        .wgpu_render_state
        .clone()
        .map_or_else(Renderer::default, Renderer::with_gpu),
      input,
      screen: None,
      system_palette,
      palette,
      font_size,
      capture: args.capture,
      #[cfg(feature = "bench-events")]
      events,
      #[cfg(feature = "bench-events")]
      mark: None,
      trace,
    })
  }

  fn receive_frame(
    &mut self,
    ctx: &egui::Context,
    screen: Screen,
  ) -> (bool, bool) {
    let mut content = false;
    let mut received = false;
    if let Some(mut frame) = self.bridge.take() {
      #[cfg(target_os = "macos")]
      if let Some(native) = &mut self.native {
        native.update(&frame.ui, std::mem::take(&mut frame.effects));
      }
      self.input.snapshot(std::sync::Arc::clone(&frame.ui));
      ctx.send_viewport_cmd(ViewportCommand::Title(frame.title.clone()));
      if frame.quit {
        ctx.send_viewport_cmd(ViewportCommand::Close);
      }
      if let Some(text) = &frame.clipboard {
        ctx.copy_text(text.clone());
      }
      let matches = self.font_size.to_bits() == frame.ui_font_size.to_bits()
        && frame.screen == screen;
      if self.font_size.to_bits() != frame.ui_font_size.to_bits() {
        self.font_size = frame.ui_font_size;
        self.screen = None;
        ctx.request_repaint();
      }
      if matches {
        content = frame.content;
        received = true;
        self.frame = Some(frame);
      } else if let Some(previous) = &mut self.frame {
        previous.reloaded.append(&mut frame.reloaded);
      }
    }
    (content, received)
  }

  fn send_input(&mut self, input: Input) {
    self.trace.input();
    self.bridge.send(input);
  }

  #[cfg(target_os = "macos")]
  fn receive_documents(&mut self, ctx: &egui::Context) {
    for event in self.documents.drain() {
      match event {
        fmv_macos_events::Event::Open(paths) => {
          for path in paths {
            self.send_input(Input::Open(path));
          }
        }
        fmv_macos_events::Event::Quit => {
          ctx.send_viewport_cmd(ViewportCommand::Close);
        }
      }
    }
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
    self.refresh_system(&ctx);
    #[cfg(target_os = "macos")]
    if let Some(preferences) = self.preferences.take() {
      self.renderer.display(preferences);
      self.bridge.send(Input::ReduceMotion(preferences.motion));
      ctx.all_styles_mut(|style| {
        style.animation_time = if preferences.motion {
          0.0
        } else {
          egui::Style::default().animation_time
        };
      });
    }
    let scale = ctx.pixels_per_point();
    let screen = vpdf_gui::fonts::screen(
      &ctx,
      ui.max_rect().size(),
      self.font_size,
      scale,
    );
    if self.screen != Some(screen) {
      self.screen = Some(screen);
      self.bridge.send(Input::Resize(screen));
    }
    #[cfg(target_os = "macos")]
    self.receive_documents(&ctx);
    #[cfg(target_os = "macos")]
    if let Some(native) = &self.native {
      for input in native.drain() {
        self.bridge.send(input);
      }
    }
    let (content, received) = self.receive_frame(&ctx, screen);
    if let Some(frame) = &self.frame {
      self.palette = paint::scene_theme(&frame.scene).palette();
      set_theme(&ctx, self.palette);
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
      #[cfg(target_os = "macos")]
      if let Some(command) = events.take_native()
        && let Some(native) = &mut self.native
      {
        native.probe(&command);
      }
    }
  }

  fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
    let [r, g, b] = self.palette.background;
    egui::Color32::from_rgb(r, g, b).to_normalized_gamma_f32()
  }
  fn on_exit(&mut self) {
    #[cfg(target_os = "macos")]
    if let Some(native) = &mut self.native {
      native.cancel();
    }
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
  let config = args.config.load();
  let options = options(&args)
    .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
  #[cfg(target_os = "macos")]
  let (_registration, documents) = fmv_macos_events::Registration::install();
  eframe::run_native(
    "Vpdf",
    options,
    Box::new(move |cc| {
      Ok(Box::new(Window::new(
        cc,
        args,
        config,
        #[cfg(target_os = "macos")]
        documents,
      )?))
    }),
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn page_option_is_unknown() {
    let error = Args::try_parse_from(["vpdf-gui", "--page", "2", "book.pdf"])
      .err()
      .unwrap();
    assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
  }

  #[test]
  fn system_input_selects_palette() {
    use vpdf_model::theme::Palette;
    let ctx = egui::Context::default();
    for (system, expected) in [
      (Some(egui::Theme::Light), Palette::light()),
      (Some(egui::Theme::Dark), Palette::dark()),
      (Some(egui::Theme::Light), Palette::light()),
      (None, Palette::dark()),
    ] {
      let mut output = ctx.run_ui(
        egui::RawInput { system_theme: system, ..Default::default() },
        |ui| {
          let ctx = ui.ctx();
          assert_eq!(system_palette(ctx), expected);
          for (palette, theme) in [
            (Palette::light(), egui::Theme::Light),
            (Palette::dark(), egui::Theme::Dark),
          ] {
            set_theme(ctx, palette);
            assert_eq!(ctx.theme(), theme);
            assert_eq!(system_palette(ctx), expected);
          }
        },
      );
      assert!(output.viewport_output.values().all(|viewport| {
        viewport.commands.iter().all(|command| {
          !matches!(
            command,
            ViewportCommand::SetTheme(
              egui::SystemTheme::Light | egui::SystemTheme::Dark
            )
          )
        })
      }));
      output.textures_delta.clear();
    }
  }

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
