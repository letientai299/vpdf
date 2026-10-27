use crate::Result;
use mupdf::pdf::{PdfDocument, PdfObject};
use vpdf_model::theme::Palette;

// Match the authored template's semantic swatches.
const SWATCHES: [&str; 8] = [
  "0.0627451 0.1254902 0.1882353",
  "0.1254902 0.1254902 0.1882353",
  "0.1882353 0.1254902 0.1882353",
  "0.2509804 0.1254902 0.1882353",
  "0.3137255 0.1254902 0.1882353",
  "0.3764706 0.1254902 0.1882353",
  "0.4392157 0.1254902 0.1882353",
  "0.5019608 0.1254902 0.1882353",
];

pub(super) fn rgb(color: [u8; 3]) -> String {
  color.map(|value| format!("{:.6}", f32::from(value) / 255.0)).join(" ")
}

pub(super) fn authored(native: &PdfDocument, palette: Palette) -> Result<()> {
  for page in 0..native.page_count()? {
    if let Some(contents) = native.find_page(page)?.get_dict("Contents")? {
      recolor(contents, palette)?;
    }
  }
  Ok(())
}

fn recolor(mut contents: PdfObject, palette: Palette) -> Result<()> {
  if contents.is_array()? {
    for stream in contents.array_iter()? {
      recolor(stream?, palette)?;
    }
    return Ok(());
  }
  let mut text = String::from_utf8(contents.read_stream()?)?;
  let colors = [
    palette.background,
    palette.foreground,
    palette.accent,
    palette.r#match,
    palette.link,
    palette.highlight,
    palette.highlight,
    palette.link,
  ];
  for (swatch, color) in SWATCHES.into_iter().zip(colors) {
    for operator in ["scn", "SCN"] {
      text = text.replace(
        &format!("{swatch} {operator}"),
        &format!("{} {operator}", rgb(color)),
      );
    }
  }
  contents.write_stream_string(&text)?;
  Ok(())
}
