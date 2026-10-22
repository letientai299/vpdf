use super::record::Record;
use crate::jobs::figures::Entries;
use crate::text::{Bounds, PageId};
use std::path::{Path, PathBuf};
use vpdf_model::document::{Figure, FigureSource};

type Row = (PageId, [f32; 4], String, bool);

pub(crate) struct FigureCache(Record);

impl FigureCache {
  pub(crate) fn open(
    root: PathBuf,
    source: &Path,
    password: &str,
    ready: impl FnMut() -> bool,
  ) -> Option<Self> {
    Record::open(
      root,
      source,
      password,
      "figures",
      env!("VPDF_FIGURE_CODE"),
      ready,
    )
    .map(Self)
  }

  pub(crate) fn read(&self, pages: usize) -> Option<Entries> {
    let (count, rows): (usize, Vec<Row>) =
      serde_json::from_slice(&self.0.read()?).ok()?;
    if count != pages || rows.len() > 100_000 {
      return None;
    }
    rows
      .into_iter()
      .map(|(page, bounds, caption, graphic)| {
        let [x0, y0, x1, y1] = bounds;
        (page.0 < pages
          && bounds.iter().all(|n| n.is_finite())
          && x0 < x1
          && y0 < y1)
          .then_some((
            page,
            Figure {
              bounds: Bounds { x0, y0, x1, y1 },
              caption,
              source: if graphic {
                FigureSource::Graphic
              } else {
                FigureSource::Caption
              },
            },
          ))
      })
      .collect()
  }

  pub(crate) fn save(&self, entries: &Entries, pages: usize) {
    if entries.len() > 100_000 {
      return;
    }
    let rows: Vec<_> = entries
      .iter()
      .map(|(page, figure)| {
        let b = figure.bounds;
        (
          page,
          [b.x0, b.y0, b.x1, b.y1],
          &figure.caption,
          figure.source == FigureSource::Graphic,
        )
      })
      .collect();
    if let Ok(bytes) = serde_json::to_vec(&(pages, rows)) {
      let _ = self.0.write(&bytes);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::content_cache::store::trim;
  use anyhow::Context;

  fn entries() -> Entries {
    vec![(
      PageId(1),
      Figure {
        bounds: Bounds { x0: 20.0, y0: 30.0, x1: 200.0, y1: 150.0 },
        caption: "FIGURE 4.1 Architecture of a CUDA-capable GPU.".into(),
        source: FigureSource::Graphic,
      },
    )]
  }

  #[test]
  fn figures_roundtrip_and_invalidate() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.pdf");
    std::fs::write(&source, b"first PDF bytes")?;
    let cache = FigureCache::open(root.path().into(), &source, "", || true)
      .context("cache")?;
    cache.save(&entries(), 2);
    let found = cache.read(2).context("figures")?;
    assert_eq!(found[0].0, PageId(1));
    assert_eq!(found[0].1.caption, entries()[0].1.caption);
    assert_eq!(found[0].1.bounds, entries()[0].1.bounds);
    assert_eq!(found[0].1.source, FigureSource::Graphic);
    assert!(cache.read(3).is_none());
    let copy = root.path().join("copy.pdf");
    std::fs::copy(&source, &copy)?;
    let copied = FigureCache::open(root.path().into(), &copy, "", || true)
      .context("copy")?;
    assert_eq!(cache.0.path(), copied.0.path());
    assert!(copied.read(2).is_some());
    let stale = Record::open(
      root.path().into(),
      &source,
      "",
      "figures",
      "other code",
      || true,
    )
    .context("stale")?;
    assert!(stale.read().is_none());
    assert!(
      FigureCache::open(root.path().into(), &source, "secret", || true)
        .is_none()
    );
    std::fs::write(&source, b"other PDF bytes")?;
    assert!(cache.read(2).is_none());
    let changed = FigureCache::open(root.path().into(), &source, "", || true)
      .context("changed")?;
    assert!(changed.read(2).is_none());
    cache.save(&entries(), 2);
    assert!(changed.read(2).is_none());
    trim(root.path(), 0)?;
    assert!(!cache.0.path().exists());
    Ok(())
  }

  #[test]
  fn invalid_figure_records_miss() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.pdf");
    std::fs::write(&source, b"PDF bytes")?;
    let cache = FigureCache::open(root.path().into(), &source, "", || true)
      .context("cache")?;
    cache.save(&Vec::new(), 2);
    assert!(cache.read(2).context("empty")?.is_empty());
    cache.save(&entries(), 2);
    let path = cache.0.path();
    let bytes = std::fs::read(&path)?;
    for len in 0..bytes.len() {
      std::fs::write(&path, &bytes[..len])?;
      assert!(cache.read(2).is_none(), "{len}");
    }
    let mut damaged = bytes;
    damaged[32] ^= 1;
    std::fs::write(&path, damaged)?;
    assert!(cache.read(2).is_none());
    for (page, area) in [
      (2, [20.0, 30.0, 200.0, 150.0]),
      (0, [200.0, 30.0, 20.0, 150.0]),
      (0, [20.0, 30.0, 200.0, 30.0]),
    ] {
      let rows: Vec<Row> = vec![(PageId(page), area, String::new(), false)];
      cache.0.write(&serde_json::to_vec(&(2, rows))?)?;
      assert!(cache.read(2).is_none());
    }
    Ok(())
  }
}
