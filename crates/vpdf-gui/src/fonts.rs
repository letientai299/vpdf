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
  let data = data(source.select_by_postscript_name(name).ok()?)?;
  Some(
    crate::collection::compact(data.bytes(), data.index)
      .map_or(data, egui::FontData::from_owned),
  )
}

#[cfg(target_os = "macos")]
fn data(handle: font_kit::handle::Handle) -> Option<egui::FontData> {
  use font_kit::handle::Handle;
  match handle {
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
  fn collections_preserve_rasters() {
    let source = font_kit::source::SystemSource::new();
    for name in
      ["STHeitiSC-Light", "HiraginoSans-W3", "AppleSDGothicNeo-Regular"]
    {
      let original =
        data(source.select_by_postscript_name(name).unwrap()).unwrap();
      let packed =
        crate::collection::compact(original.bytes(), original.index).unwrap();
      assert!(packed.capacity() < original.bytes().len());
      assert!(crate::collection::compact(&packed, 0).is_none());
      assert!(crate::collection::compact(&packed, 1).is_none());
      let mut invalid = packed.clone();
      invalid[28..32].copy_from_slice(b"DSIG");
      assert!(crate::collection::compact(&invalid, 0).is_none());
      invalid[28..32].copy_from_slice(&packed[28..32]);
      invalid[36..40].copy_from_slice(&u32::MAX.to_be_bytes());
      assert!(crate::collection::compact(&invalid, 0).is_none());
      assert!(
        crate::collection::compact(&packed[..packed.len() - 4], 0).is_none()
      );
      assert_eq!(
        raster(original),
        raster(egui::FontData::from_owned(packed)),
        "{name}"
      );
    }
  }

  fn raster(data: egui::FontData) -> (Vec<egui::ColorImage>, egui::Vec2) {
    let ctx = egui::Context::default();
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("test".into(), data.into());
    fonts
      .families
      .get_mut(&egui::FontFamily::Monospace)
      .unwrap()
      .push("test".into());
    ctx.set_fonts(fonts);
    let mut size = egui::Vec2::ZERO;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
      size = ui.fonts_mut(|fonts| {
        fonts
          .layout_no_wrap(
            "中文東京ひらがな한글가힣Tiếng Việt".into(),
            egui::FontId::monospace(FONT_SIZE),
            egui::Color32::WHITE,
          )
          .size()
      });
    });
    assert_eq!(output.textures_delta.set.len(), 1);
    let images = output
      .textures_delta
      .set
      .values()
      .flatten()
      .map(|delta| {
        let egui::ImageData::Color(image) = &delta.image;
        (**image).clone()
      })
      .collect();
    output.textures_delta.clear();
    (images, size)
  }

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
