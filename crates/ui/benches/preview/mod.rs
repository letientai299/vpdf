use anyhow::Result;
use std::hint::black_box;
use vpdf_model::tile::TileKey;
use vpdf_ui::layout::visible;
use vpdf_ui::source::Raster;

struct Prepared {
  key: TileKey,
  pixels: Vec<u8>,
}

impl Raster for Prepared {
  fn raster(
    &mut self,
    key: TileKey,
    visit: impl FnOnce(&[u8], usize) -> Result<()>,
  ) -> Result<()> {
    anyhow::ensure!(key == self.key, "Unexpected prepared raster");
    visit(&self.pixels, key.width as usize * 3)
  }
}

pub fn compose() -> Result<()> {
  let geometry: super::Geometry = super::fixture("geometry")?;
  let layout =
    super::layouts::tiles(geometry.pages.into_iter().map(super::bounds));
  let (manifest, pixels) = super::raster::fixture()?;
  let key = visible(&layout, 0.0)[0];
  anyhow::ensure!(key == manifest.key(), "Prepared raster dimensions changed");
  let mut prepared = Prepared { key, pixels };
  let output = vpdf_ui::preview::image(&layout, key, &mut prepared)?;
  assert_eq!(output.pixels().len() / 4, prepared.pixels.len() / 3);
  for (actual, expected) in output
    .pixels()
    .as_chunks::<4>()
    .0
    .iter()
    .zip(prepared.pixels.as_chunks::<3>().0)
  {
    assert_eq!(&actual[..3], expected);
    assert_eq!(actual[3], 255);
  }
  println!("preview output {}", super::hash::digest(output.pixels()));
  super::measure("preview::image prepared strip", || {
    black_box(
      vpdf_ui::preview::image(&layout, key, &mut prepared)
        .expect("prepared preview"),
    );
  })
}
