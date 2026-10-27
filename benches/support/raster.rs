use anyhow::{Context, Result};

#[derive(serde::Deserialize)]
pub struct Manifest {
  pub width: i32,
  pub height: i32,
  pub rgb_sha256: String,
  pub source_sha256: String,
  pub revision: String,
  page: usize,
  scale: u32,
  rotation: u16,
  x: i32,
  y: i32,
  crop: [u16; 4],
}

impl Manifest {
  pub fn key(&self) -> vpdf_model::tile::TileKey {
    vpdf_model::tile::TileKey {
      page: vpdf_model::text::PageId(self.page),
      scale: self.scale,
      rotation: self.rotation,
      x: self.x,
      y: self.y,
      width: self.width,
      height: self.height,
      crop: vpdf_model::geometry::Crop(self.crop),
      recolor: vpdf_model::recolor::Recolor::default(),
    }
  }
}

pub fn fixture() -> Result<(Manifest, Vec<u8>)> {
  let directory = super::directory::fixtures();
  let metadata = std::fs::read(directory.join("manifest.json"))
    .context("Run mise run bench:fixtures first")?;
  let manifest: Manifest = serde_json::from_slice(&metadata)?;
  let pixels = std::fs::read(directory.join("tile.rgb"))?;
  anyhow::ensure!(
    manifest.width > 0
      && manifest.height > 0
      && pixels.len() as i64
        == i64::from(manifest.width) * i64::from(manifest.height) * 3,
    "Invalid raster fixture dimensions"
  );
  anyhow::ensure!(
    super::hash::digest(&pixels) == manifest.rgb_sha256,
    "Raster fixture hash mismatch"
  );
  println!(
    "fixture {} source {} revision {}",
    manifest.rgb_sha256, manifest.source_sha256, manifest.revision
  );
  Ok((manifest, pixels))
}
