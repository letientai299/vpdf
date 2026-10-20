use eframe::egui;

pub fn install(ctx: &egui::Context) {
  let fonts = egui::FontDefinitions::default();
  #[cfg(target_os = "macos")]
  let fonts = installed(fonts);
  ctx.set_fonts(fonts);
}

#[cfg(target_os = "macos")]
fn installed(mut fonts: egui::FontDefinitions) -> egui::FontDefinitions {
  use font_kit::source::SystemSource;
  let source = SystemSource::new();
  for name in ["STHeitiSC-Light", "HiraginoSans-W3", "AppleSDGothicNeo-Regular"]
  {
    let Some(font) = load(&source, name) else { continue };
    fonts.font_data.insert(name.into(), font.into());
    fonts
      .families
      .entry(egui::FontFamily::Monospace)
      .or_default()
      .push(name.into());
  }
  fonts
}

#[cfg(target_os = "macos")]
fn load(
  source: &font_kit::source::SystemSource,
  name: &str,
) -> Option<egui::FontData> {
  use font_kit::handle::Handle;
  match source.select_by_postscript_name(name).ok()? {
    Handle::Path { path, font_index } => {
      let mut data = egui::FontData::from_owned(std::fs::read(path).ok()?);
      data.index = font_index;
      Some(data)
    }
    Handle::Memory { bytes, font_index } => {
      Some(egui::FontData::from_blob(bytes, font_index))
    }
  }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
  use super::*;
  use crate::paint::FONT_SIZE;

  #[test]
  fn fallback_covers_cjk() {
    let ctx = egui::Context::default();
    let font = egui::FontId::monospace(FONT_SIZE);
    let mut baseline = None;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
      baseline = Some(ui.fonts_mut(|fonts| {
        assert!(!fonts.has_glyphs(&font, "中文東京한글"));
        (fonts.glyph_width(&font, 'm'), fonts.row_height(&font))
      }));
    });
    output.textures_delta.clear();
    install(&ctx);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
      ui.fonts_mut(|fonts| {
        assert!(fonts.has_glyphs(&font, "中文東京ひらがな한글Tiếng Việt"));
        assert_eq!(
          (fonts.glyph_width(&font, 'm'), fonts.row_height(&font)),
          baseline.unwrap()
        );
      });
    });
    output.textures_delta.clear();
    assert!(
      load(&font_kit::source::SystemSource::new(), "vpdf-no-such-font")
        .is_none()
    );
  }
}
