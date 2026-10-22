use super::common::SeedReader;
use crate::harness::{corpus, solo};
use mupdf::{Colorspace, Matrix};
use std::path::PathBuf;
use vpdf::Result;
use vpdf::solo::Solo;
use vpdf_model::text::PageId;
use vpdf_model::tile::TileKey;
use vpdf_pdf::document::Document;
use vpdf_pdf::jobs::Wake;
use vpdf_pdf::raster::raster;

#[test]
#[ignore = "requires verified public PDF corpus"]
fn tiles_equal_full_page_pixels() -> Result<()> {
  for file in ["paper.pdf", "book.pdf", "form.pdf", "ocr.pdf"] {
    let mut doc = Document::open(&corpus(file)?, "")?;
    let page = doc.native.load_page(0)?;
    let full = page.to_pixmap(
      &Matrix::IDENTITY,
      &Colorspace::device_rgb(),
      false,
      true,
    )?;
    let key = TileKey {
      scale: 1024,
      x: 32,
      y: 64,
      width: 256,
      height: 128,
      ..TileKey::default()
    };
    let tile = raster(&mut doc, key)?;
    for row in 0..key.height as usize {
      let start =
        ((key.y as usize + row) * full.width() as usize + key.x as usize) * 3;
      let tile_start = row * key.width as usize * 3;
      assert_eq!(
        &tile[tile_start..tile_start + key.width as usize * 3],
        &full.samples()[start..start + key.width as usize * 3],
        "{file}"
      );
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn crop_preserves_body_ink() -> Result<()> {
  for file in
    ["paper.pdf", "book.pdf", "form.pdf", "ocr.pdf", "scan.pdf", "review.pdf"]
  {
    let mut app = solo(file, false, crate::common::cells(80, 50))?;
    app.toggle_crop()?;
    crate::common::crop_loaded(&mut app)?;
    let crop = app.layout.crop_for(PageId(0));
    assert!(crop.0.iter().all(|trim| *trim < 400), "{file}: {crop:?}");
    if matches!(file, "paper.pdf" | "book.pdf") {
      assert!(crop.0[0] + crop.0[2] > 100, "{file} side margins: {crop:?}");
    }
    if file == "paper.pdf" {
      assert!(crop.0[0].abs_diff(crop.0[2]) <= 10, "{crop:?}");
      assert!(crop.0[0] > 140, "{crop:?}");
    }
    for page in 0..app.doc.pages.len() {
      app.command(&(page + 1).to_string())?;
      app.crop_updates()?;
      crate::common::crop_loaded(&mut app)?;
      let crop = app.layout.crop_for(PageId(page));
      let doc = &mut app.doc;
      let cropped = crop.apply(doc.pages[page].bounds);
      let mut stamp = None;
      for line in &doc.text(PageId(page))?.lines {
        if file == "paper.pdf" && page == 0 && line.text().starts_with("arXiv:")
        {
          stamp = line.span();
          assert!(stamp.unwrap().x1 < cropped.x0);
          continue;
        }
        for glyph in line.glyphs.iter().filter(|g| !g.ch.is_whitespace()) {
          let (x, y) = (
            f32::midpoint(glyph.bounds.x0, glyph.bounds.x1),
            f32::midpoint(glyph.bounds.y0, glyph.bounds.y1),
          );
          assert!(
            cropped.contains(x, y),
            "{file} p{}: {:?}",
            page + 1,
            glyph.ch
          );
        }
      }
      check_crop_pixels(doc, PageId(page), crop, stamp)?;
    }
  }
  Ok(())
}

fn check_crop_pixels(
  doc: &mut Document,
  page: PageId,
  crop: vpdf_ui::layout::Crop,
  stamp: Option<vpdf_model::text::Bounds>,
) -> Result<()> {
  use vpdf_ui::layout::{page_matrix, transform};
  let bounds = doc.pages[page.0].bounds;
  let scale = (320.0 / bounds.width() * 1024.0) as u32;
  let matrix = page_matrix(bounds, scale, 0);
  let cropped = transform(crop.apply(bounds), &matrix);
  let stamp = stamp.map(|stamp| transform(stamp, &matrix));
  let pixmap = doc.display(page)?.to_pixmap(
    &vpdf_pdf::raster::matrix(&matrix),
    &Colorspace::device_gray(),
    false,
  )?;
  let stride = usize::try_from(pixmap.stride())?;
  for (y, row) in pixmap.samples().chunks(stride).enumerate() {
    for (x, value) in row[..pixmap.width() as usize].iter().enumerate() {
      if *value >= 235 {
        continue;
      }
      let (px, py) = (
        x as f32 + pixmap.x() as f32 + 0.5,
        y as f32 + pixmap.y() as f32 + 0.5,
      );
      assert!(
        cropped.contains(px, py)
          || stamp.is_some_and(|stamp| stamp.contains(px, py)),
        "p{} lost ink at {px},{py}: {crop:?}",
        page.0 + 1
      );
    }
  }
  Ok(())
}

#[test]
fn crop_ignores_margin_stamps() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("stamps.pdf");
  for (matrix, label, graphic, trimmed) in [
    ("0 1 -1 0 20 40", "arXiv:1706.03762", "", (true, true)),
    ("0 1 -1 0 290 40", "arXiv:1706.03762", "", (true, true)),
    ("0 -1 1 0 10 160", "arXiv:1706.03762", "", (true, true)),
    ("0 1 -1 0 20 40", "Important label", "", (false, true)),
    ("1 0 0 1 10 20", "arXiv:1706.03762", "", (false, true)),
    ("0 1 -1 0 55 40", "arXiv:1706.03762", "", (false, true)),
    ("0 1 -1 0 20 40", "arXiv:1706.03762", "12 80 5 5 re f", (false, true)),
    ("0 1 -1 0 20 40", "arXiv:1706.03762", "6 80 1 5 re f", (false, true)),
    (
      "0 1 -1 0 20 40",
      "arXiv:1706.03762",
      "BT /F1 6 Tf 1 0 0 1 12 80 Tm (x) Tj ET",
      (false, true),
    ),
    (
      "0 1 -1 0 20 40",
      "arXiv:1706.03762",
      "q 5 0 0 5 12 80 cm BI /W 1 /H 1 /CS /RGB /BPC 8 /F /AHx ID 000000> EI Q",
      (false, true),
    ),
  ] {
    let content = format!(
      "60 40 180 120 re f BT /F1 10 Tf {matrix} Tm ({label}) Tj ET {graphic}"
    );
    crate::common::write_pdf(
      &path,
      &[
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] \
       /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
          .into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        format!(
          "<< /Length {} >>\nstream\n{content}\nendstream",
          content.len()
        ),
      ],
      "",
    )?;
    let mut doc = Document::open(&path, "")?;
    let crop = vpdf_pdf::crop::margins(&mut doc)?[0];
    assert_eq!(doc.cache_bytes(), 0, "margin detection filled the text cache");
    assert_eq!(
      crop.0[0] > 170,
      trimmed.0,
      "{matrix}: {label}: {graphic}: {crop:?}"
    );
    assert_eq!(
      crop.0[2] > 170,
      trimmed.1,
      "{matrix}: {label}: {graphic}: {crop:?}"
    );
  }
  Ok(())
}

/// Cropping keeps visible display lists cached.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn crop_samples_once() -> Result<()> {
  use vpdf::app::App;
  let screen = crate::common::cells(60, 50);
  let mut app = solo("book.pdf", false, screen)?;
  app.command("100")?;
  let lists = app.doc.lists();
  // Display lists a frame interprets, as the render worker shares them.
  let frame = |app: &mut App| -> Result<usize> {
    let mut interpreted = 0;
    for page in app.layout.visible(app.reader.scroll).map(PageId) {
      interpreted += usize::from(lists.lock().unwrap().get(&page).is_none());
      app.doc.display(page)?;
    }
    Ok(interpreted)
  };
  frame(&mut app)?;
  let kept: Vec<_> =
    app.layout.visible(app.reader.scroll).map(PageId).collect();
  let mut interpreted = Vec::new();
  for _ in 0..3 {
    app.toggle_crop()?;
    crate::common::crop_loaded(&mut app)?;
    assert!(kept.iter().all(|page| lists.lock().unwrap().contains(page)));
    interpreted.push(frame(&mut app)?);
  }
  assert_eq!(interpreted[1..], [0, 0]);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn inline_page_fits_requested_cells() -> Result<()> {
  let mut doc = Document::open(&corpus("paper.pdf")?, "")?;
  let mut out = Vec::new();
  vpdf::inline::print(&mut doc, PageId(0), (40, 12), &mut out)?;
  let text = String::from_utf8(out)?;
  let rows: Vec<_> = text.split('\n').collect();
  assert!(rows.len() <= 12 && !text.ends_with('\n'));
  let cells = rows[0].chars().filter(|c| *c == '\u{10EEEE}').count();
  assert!(cells > 0 && cells <= 40);
  assert!(text.starts_with("\x1b_Ga=T,U=1"));
  Ok(())
}

/// Tiles from the render worker match a full-page render: text pages
/// drawn in bands at any scale, image pages drawn whole up to 8 MP.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn worker_tiles_equal_full_page_pixels() -> Result<()> {
  use vpdf::raster::Raster;
  use vpdf_ui::layout::page_matrix;
  let text = [1024, 2048, 3277];
  for (file, scales) in [
    ("paper.pdf", text),
    ("book.pdf", text),
    ("form.pdf", text),
    ("ocr.pdf", [900, 1024, 1300]),
  ] {
    let path = corpus(file)?;
    let doc = Document::open(&path, "")?;
    for scale in scales {
      let bounds = doc.pages[0].bounds;
      let matrix = page_matrix(bounds, scale, 0);
      let page = doc.native.load_page(0)?;
      let full = page.to_pixmap(
        &vpdf_pdf::raster::matrix(&matrix),
        &Colorspace::device_rgb(),
        false,
        true,
      )?;
      let (w, h) = (full.width() as i32, full.height() as i32);
      let keys: Vec<_> = [0, 256, 512]
        .iter()
        .flat_map(|&y| [0, 512].map(|x| (x, y)))
        .filter(|&(x, y)| x < w && y < h)
        .map(|(x, y)| TileKey {
          scale,
          x,
          y,
          width: 512.min(w - x),
          height: 256.min(h - y),
          ..TileKey::default()
        })
        .collect();
      let directory = tempfile::tempdir()?;
      let spool = std::sync::Arc::new(vpdf_terminal::spool::Spool::create(
        directory.path(),
      )?);
      for encoder in [
        vpdf_terminal::payload::Encoder::default(),
        vpdf_terminal::payload::Encoder::new(Some(spool)),
      ] {
        let worker =
          Raster::start(&path, "", doc.lists(), &Wake::none(), encoder, false);
        worker.service.request(&keys, keys.len());
        for _ in &keys {
          let tile = worker
            .results
            .recv_timeout(std::time::Duration::from_secs(20))?
            .map_err(anyhow::Error::msg)?;
          assert_tile(&tile, &full, file)?;
        }
      }
    }
  }
  Ok(())
}

fn assert_tile(
  tile: &vpdf_terminal::payload::Tile,
  full: &mupdf::Pixmap,
  file: &str,
) -> Result<()> {
  use base64::Engine;
  use std::io::Read;
  let pixels = match &tile.payload {
    vpdf_terminal::payload::Payload::Encoded(encoded) => {
      let packed =
        base64::engine::general_purpose::STANDARD.decode(encoded.as_bytes())?;
      let mut pixels = Vec::new();
      flate2::read::ZlibDecoder::new(&packed[..]).read_to_end(&mut pixels)?;
      pixels
    }
    vpdf_terminal::payload::Payload::Spooled { spool, slot } => {
      use std::os::unix::ffi::OsStringExt;
      use std::os::unix::fs::FileExt;
      let path =
        base64::engine::general_purpose::STANDARD.decode(&spool.payload)?;
      let path = PathBuf::from(std::ffi::OsString::from_vec(path));
      let keys = vpdf_terminal::spool::Spool::keys(*slot);
      let offset: u64 = keys.split("O=").nth(1).unwrap().parse()?;
      let mut pixels = vec![0; slot.len];
      std::fs::File::open(path)?.read_exact_at(&mut pixels, offset)?;
      pixels
    }
  };
  let key = tile.key;
  let stride = full.stride() as usize;
  let row = key.width as usize * 3;
  for y in 0..key.height as usize {
    let start = (key.y as usize + y) * stride + key.x as usize * 3;
    assert!(
      pixels[y * row..][..row] == full.samples()[start..][..row],
      "{file} scale {} tile {},{} row {y}",
      key.scale,
      key.x,
      key.y
    );
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn projected_mouse_tracks_glyphs() -> Result<()> {
  use vpdf_model::input::{MouseButton, MouseEventKind};
  use vpdf_ui::layout::{Crop, Screen};
  let screen = Screen { origin: (12, 3), ..crate::common::cells(120, 50) };
  for rotation in [0, 90, 180, 270] {
    for zoom in [0.5, 2.0] {
      let mut app = solo("paper.pdf", false, screen)?;
      let anchor = app.reader.cursor;
      let bounds = app.doc.text(anchor.page)?.glyph(anchor).unwrap().bounds;
      app.layout.restore(zoom, rotation, Crop([20, 20, 20, 20]));
      app.layout.pan_to(100_000.0);
      {
        let (app, _) = app.parts();
        app.layout.reflow(app.doc.pages.iter().map(|page| page.bounds));
      }
      app.ensure_cursor()?;
      let rect = app.layout.project(anchor.page, bounds, app.reader.scroll);
      assert!(rect.x0 >= -0.01 && rect.x1 <= screen.width() + 0.01);
      let column = (f32::midpoint(rect.x0, rect.x1) / 8.0) as u16;
      let row = (f32::midpoint(rect.y0, rect.y1) / 16.0) as u16;
      let (x, y) = app
        .layout
        .unproject(
          anchor.page,
          ((f32::from(column) + 0.5) * 8.0, (f32::from(row) + 0.5) * 16.0),
          app.reader.scroll,
        )
        .unwrap();
      let expected = app.doc.text(anchor.page)?.nearest(anchor.page, x, y);
      let down = MouseEventKind::Down(MouseButton::Left);
      let pane = app.id;
      app.mouse(crate::harness::page_route(pane, down, (column, row)))?;
      assert_eq!(app.reader.cursor, expected, "{rotation} {zoom}");
    }
  }
  Ok(())
}

fn crop_app() -> Result<(tempfile::TempDir, Solo)> {
  crate::common::temp_solo(
    |path| crate::common::text_pdf(path, &["Body text for crop testing"]),
    false,
    crate::common::cells(80, 40),
  )
}

#[test]
fn crop_commands_set_and_toggle() -> Result<()> {
  let (_directory, mut app) = crop_app()?;
  for (command, enabled) in [
    ("crop off", false),
    ("crop off", false),
    ("crop on", true),
    ("crop on", true),
    ("crop", false),
    ("crop", true),
  ] {
    app.command(command)?;
    assert_eq!(!app.layout.crop().is_none(), enabled);
    assert_eq!(app.crop_scan.active(), enabled);
  }
  let error = app.command("crop invalid").unwrap_err();
  assert_eq!(error.to_string(), "Usage: :crop [on|off]");
  assert!(app.crop_scan.active());
  assert!(!app.layout.crop().is_none());
  app.command("crop off")?;
  assert!(!app.crop_scan.active());
  assert!(app.layout.crop().is_none());
  assert!(!app.crop_updates()?);
  app.command("crop")?;
  assert!(app.crop_scan.active());
  crate::common::crop_loaded(&mut app)?;
  let crop = app.layout.crop_for(PageId(0));
  assert!(!crop.is_none());
  app.command("crop on")?;
  app.command("crop on")?;
  assert_eq!(app.layout.crop_for(PageId(0)), crop);
  assert!(!app.crop_scan.active());
  app.command("crop")?;
  assert!(app.layout.crop().is_none());
  app.command("crop on")?;
  assert_eq!(app.layout.crop_for(PageId(0)), crop);
  assert!(!app.crop_scan.active());
  app.command("crop off")?;
  app.command("crop off")?;
  assert!(app.layout.crop().is_none());
  assert!(!app.crop_scan.active());
  assert!(!app.crop_updates()?);
  Ok(())
}

#[test]
fn crop_cancel_and_cache() -> Result<()> {
  let (_directory, mut app) = crop_app()?;
  let path = app.path.clone();
  let screen = app.screen;
  let mut saved = vpdf::state::State::default();
  saved.remember(app.saved_entry().unwrap());
  app.toggle_crop()?;
  assert!(app.crop_scan.active());
  app.toggle_crop()?;
  assert!(!app.crop_scan.active() && app.layout.crop().is_none());
  assert!(!app.crop_updates()?);
  app.toggle_crop()?;
  app.command("cancel")?;
  assert!(!app.crop_scan.active() && app.layout.crop().is_none());
  assert!(!app.crop_updates()?);
  app.toggle_crop()?;
  app.restore(&saved.recent[0])?;
  assert!(!app.crop_scan.active() && app.layout.crop().is_none());
  app.toggle_crop()?;
  crate::common::crop_loaded(&mut app)?;
  assert!(!app.layout.crop().is_none());
  assert_eq!(app.layout.crops().len(), 1);
  let crop = app.layout.crop_for(PageId(0));
  app.toggle_crop()?;
  assert!(app.layout.crop_for(PageId(0)).is_none());
  app.toggle_crop()?;
  assert!(!app.crop_scan.active());
  assert_eq!(app.layout.crop_for(PageId(0)), crop);
  let mut mirrored = Solo::open(&path, "", false, screen)?;
  mirrored.mirror(&app)?;
  assert_eq!(mirrored.layout.crop_for(PageId(0)), crop);
  saved.remember(app.saved_entry().unwrap());
  let mut restored = Solo::open(&path, "", false, screen)?;
  restored.restore(&saved.recent[0])?;
  crate::common::crop_loaded(&mut restored)?;
  assert_eq!(restored.layout.crop_for(PageId(0)), crop);
  restored.reload_with("")?;
  assert!(restored.crop_scan.active());
  crate::common::crop_loaded(&mut restored)?;
  assert_eq!(restored.layout.crop_for(PageId(0)), crop);
  Ok(())
}

#[test]
fn crop_follows_rapid_space() -> Result<()> {
  use vpdf_ui::keys::{Action, Input};
  let (_directory, mut app) = crate::common::temp_solo(
    |path| {
      crate::common::pages_pdf(
        path,
        &[&["Body text for nearby crops"][..]; 100],
      )
    },
    false,
    crate::common::cells(80, 40),
  )?;
  app.toggle_crop()?;
  crate::common::crop_loaded(&mut app)?;
  assert!(app.layout.crops().len() <= 24);
  app.command("40")?;
  app.crop_updates()?;
  for _ in 0..30 {
    app.action(Input::plain(Action::Screen(1)))?;
    app.crop_updates()?;
  }
  let target = app.reader.animation().target().unwrap_or(app.reader.scroll);
  let page = app.layout.page_at(target);
  assert!(page.0 > 40);
  crate::common::crop_loaded(&mut app)?;
  let target = app.reader.animation().target().expect("crop keeps scrolling");
  assert_eq!(app.layout.page_at(target), page);
  app.reader.cancel_scroll();
  app.reader.seed_reader(|reader| reader.scroll = target);
  crate::common::crop_loaded(&mut app)?;
  let anchor = app.layout.page_at(app.reader.scroll);
  for index in anchor.0.saturating_sub(4)..(anchor.0 + 5).min(100) {
    assert!(!app.layout.crop_for(PageId(index)).is_none(), "page {index}");
  }
  assert!(app.layout.crops().len() <= 44, "unvisited pages stay lazy");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn book_body_stays_centered() -> Result<()> {
  let mut paths = vec![corpus("book.pdf")?];
  if let Some(extra) = std::env::var_os("VPDF_EXTRA") {
    paths.extend(std::env::split_paths(&extra));
  }
  let screen = crate::common::cells(80, 50);
  for path in paths {
    let mut app = Solo::open(&path, "", false, screen)?;
    app.toggle_crop()?;
    crate::common::crop_loaded(&mut app)?;
    let mut scales = Vec::new();
    for index in 182..185 {
      let page = PageId(index);
      {
        let seed_value = app.layout.pages()[index].top;
        app.reader.seed_reader(|reader| reader.scroll = seed_value);
      };
      app.crop_updates()?;
      crate::common::crop_loaded(&mut app)?;
      let text = app.doc.text(page)?;
      let body = text
        .lines
        .iter()
        .filter_map(vpdf_model::text::Line::span)
        .filter(|line| line.width() > app.doc.pages[index].bounds.width() * 0.5)
        .reduce(vpdf_model::text::Bounds::union)
        .expect("body text");
      let projected = app.layout.project(page, body, 0.0);
      let center = projected.centre().0;
      assert!(
        (center - screen.width() / 2.0).abs() < 5.0,
        "{} page {}: center {center}",
        path.display(),
        index + 1
      );
      scales.push(app.layout.pages()[index].scale);
      let crop = app.layout.crop_for(page);
      eprintln!(
        "{} page {}: center {center:.2}, scale {}, {crop:?}",
        path.display(),
        index + 1,
        app.layout.pages()[index].scale
      );
      assert!(
        vpdf_ui::layout::visible(&app.layout, app.layout.pages()[index].top)
          .iter()
          .filter(|tile| tile.page == page)
          .all(|tile| tile.crop == crop)
      );
    }
    assert!(
      scales.iter().max().unwrap().abs_diff(*scales.iter().min().unwrap()) <= 5,
      "{scales:?}"
    );
  }
  Ok(())
}
