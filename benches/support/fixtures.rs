use anyhow::{Context, Result};
use std::sync::Arc;
use vpdf_model::text::{Bounds, Glyph, Line, Text};

pub fn read<T: serde::de::DeserializeOwned>(name: &str) -> Result<T> {
  let directory = super::directory::fixtures();
  let manifest: serde_json::Value = serde_json::from_slice(
    &std::fs::read(directory.join("manifest.json"))
      .context("Run mise run bench:fixtures first")?,
  )?;
  let expected = manifest[format!("{name}_sha256")]
    .as_str()
    .context("Missing fixture hash")?;
  let bytes = std::fs::read(directory.join(format!("{name}.json")))?;
  anyhow::ensure!(
    super::hash::digest(&bytes) == expected,
    "Fixture hash mismatch: {name}"
  );
  println!(
    "fixture {} source {} revision {}",
    expected,
    manifest
      .get(format!("{name}_source_sha256"))
      .unwrap_or(&manifest["source_sha256"])
      .as_str()
      .context("Missing fixture source")?,
    manifest["revision"].as_str().context("Missing fixture revision")?
  );
  Ok(serde_json::from_slice(&bytes)?)
}

pub fn bounds([x0, y0, x1, y1]: [f32; 4]) -> Bounds {
  Bounds { x0, y0, x1, y1 }
}

#[derive(serde::Deserialize)]
struct Page {
  label: String,
  bounds: [f32; 4],
  truncated: bool,
  lines: Vec<TextLine>,
}

#[derive(serde::Deserialize)]
struct TextLine(bool, Vec<(char, [f32; 4])>);

pub struct TextPages {
  pub metadata: Vec<(String, Bounds)>,
  pub text: Vec<Arc<Text>>,
}

pub fn text() -> Result<TextPages> {
  text_named("text")
}

pub fn text_named(name: &str) -> Result<TextPages> {
  let pages: Vec<Page> = read(name)?;
  let mut prepared = TextPages { metadata: Vec::new(), text: Vec::new() };
  for page in pages {
    prepared.metadata.push((page.label, bounds(page.bounds)));
    prepared.text.push(Arc::new(Text {
      truncated: page.truncated,
      lines: page
        .lines
        .into_iter()
        .map(|TextLine(paragraph, glyphs)| Line {
          paragraph,
          glyphs: glyphs
            .into_iter()
            .map(|(ch, area)| Glyph { ch, bounds: bounds(area) })
            .collect(),
        })
        .collect(),
    }));
  }
  Ok(prepared)
}
