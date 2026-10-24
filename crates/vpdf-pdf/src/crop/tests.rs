use super::*;
#[test]
fn page_centers_keep_width() {
  let boxes = [
    Some([0.21, 0.2, 0.86, 0.9]),
    Some([0.14, 0.1, 0.79, 0.9]),
    Some([0.21, 0.3, 0.86, 0.9]),
    Some([0.45, 0.4, 0.55, 0.6]),
    Some([0.02, 0.1, 0.98, 0.9]),
    None,
  ];
  let crops = trims(&boxes);
  let page = Bounds { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 };
  for (index, bounds) in boxes.iter().enumerate() {
    let shown = crops[index].apply(page);
    if let Some([left, top, right, bottom]) = bounds {
      assert!(shown.contains(*left, *top) && shown.contains(*right, *bottom));
      assert!((shown.centre().0 - f32::midpoint(*left, *right)).abs() < 0.002);
    }
    if index != 4 {
      assert!((shown.width() - (0.68 + 2.0 / WIDTH)).abs() < 0.003);
    }
  }
  assert_eq!(trims(&[None, None]), vec![Crop::default(); 2]);
}

fn fixture(content: &str) -> Result<Ink> {
  let mut native = mupdf::pdf::PdfDocument::new();
  let page = native.new_page((540, 666))?;
  let resources = native.new_object_from_str(
    "<< /Font << /F1 << /Type /Font /Subtype /Type1 \
     /BaseFont /Helvetica >> >> >>",
  )?;
  let stream = native.add_stream(
    &mupdf::Buffer::from_copied_bytes(content.as_bytes())?,
    None,
    false,
  )?;
  let mut object = page.object();
  object.dict_put("Resources", resources)?;
  object.dict_put("Contents", stream)?;
  let list = page.to_display_list(true)?;
  ink_list(&list, crate::document::rect(page.bounds()?))
}

const BODY: &str = "BT /F1 12 Tf 84 550 Td (Body content) Tj ET \
                    84 100 372 400 re f";
const LEFT: &str = "-12 640 564 0.5 re f 0 640 120 8 re f";
const RIGHT: &str = "-12 640 564 0.5 re f 420 640 120 8 re f";

#[test]
fn repeated_tabs_trim_margins() -> Result<()> {
  let left = fixture(&format!("{LEFT} {BODY}"))?;
  let right = fixture(&format!("{RIGHT} {BODY}"))?;
  assert_eq!(confirmed(std::slice::from_ref(&left)), 0);
  let samples = [left, right];
  assert_eq!(confirmed(&samples), 1);
  let boxes: Vec<_> = samples.iter().map(|ink| ink.bounds(1)).collect();
  for crop in trims(&boxes) {
    assert!(crop.0[0] > 130 && crop.0[2] > 130, "{crop:?}");
  }
  let lone = trims(&[samples[0].bounds(0)])[0];
  assert_eq!(lone.0[0] + lone.0[2], 0);
  Ok(())
}

#[test]
fn footer_requires_corroboration() -> Result<()> {
  let header = fixture(&format!("{LEFT} {BODY}"))?;
  let footer = fixture(&format!("-12 25 564 0.5 re f 0 25 120 8 re f {BODY}"))?;
  assert_eq!(footer.bands, 2);
  assert_eq!(confirmed(&[header, footer.clone()]), 0);
  assert_eq!(confirmed(&[footer.clone(), footer.clone()]), 2);
  assert_eq!(footer.bounds(1), footer.raw);
  assert!(footer.bounds(2).unwrap()[0] > 0.15);
  Ok(())
}

#[test]
fn plaque_preserves_white_text() -> Result<()> {
  for (x, right) in [(366, 0.9), (470, 1.0)] {
    let ink = fixture(&format!(
      "84 586 468 0.5 re f 357 586 10 31 re f \
       357 586 183 31 re f \
       1 g BT /F1 12 Tf {x} 596 Td (CHAPTER) Tj ET 0 g {BODY}"
    ))?;
    assert_eq!(ink.bands, 1);
    let bounds = ink.bounds(1).unwrap();
    assert!(bounds[2] < right && bounds[1] < 0.11, "{bounds:?}");
    assert!(bounds[2] >= (x as f32 + 52.0) / 540.0, "{bounds:?}");
  }
  Ok(())
}

#[test]
fn figures_protect_edges() -> Result<()> {
  for graphic in [
    "-12 590 564 0.5 re f",
    "0 100 100 400 re f",
    "0 620 100 30 re f",
    "q 100 0 0 30 0 625 cm BI /W 1 /H 1 /CS /RGB \
     /BPC 8 /F /AHx ID 000000> EI Q",
    "BT /F1 12 Tf 0 600 Td (Margin note) Tj ET",
  ] {
    let ink = fixture(&format!("{LEFT} {BODY} {graphic}"))?;
    let bounds = ink.bounds(1).unwrap();
    assert!(bounds[0] < 0.01, "{graphic}: {bounds:?}");
  }
  Ok(())
}

#[test]
fn ambiguous_shapes_keep_margins() -> Result<()> {
  for decoration in [
    "-12 640 564 0.5 re f",
    "0 640 120 8 re f",
    "-12 640 564 0.5 re f 0 640 m 120 640 l 0 648 l h f",
  ] {
    let ink = fixture(&format!("{decoration} {BODY}"))?;
    assert_eq!(ink.bands, 0, "{decoration}");
    assert_eq!(ink.bounds(1), ink.raw);
  }
  let scan = fixture(&format!("{LEFT} 84 100 372 400 re f"))?;
  assert_eq!(scan.bands, 0);
  Ok(())
}

#[test]
fn cover_keeps_own_bounds() {
  let cover = Some([0.0, 0.0, 1.0, 1.0]);
  let body = Some([0.155, 0.05, 0.845, 0.91]);
  let crops = trims(&[cover, body, body, None]);
  assert!(crops[0].is_none());
  assert!(crops[1].0[0] > 130 && crops[1].0[1] > 20);
  assert_eq!(crops[1], crops[2]);
  assert_eq!(crops[2], crops[3]);
  let later = Some([0.05, 0.02, 0.95, 0.98]);
  let crop = calibrated(&[later], &[cover, body, body])[0];
  let page = Bounds { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 };
  let shown = crop.apply(page);
  assert!(shown.contains(0.05, 0.02) && shown.contains(0.95, 0.98));
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn document_preserves_content() -> Result<()> {
  let path = match std::env::var_os("VPDF_CROP_INPUT") {
    Some(path) => std::path::PathBuf::from(path),
    None => {
      std::path::PathBuf::from(std::env::var("VPDF_CORPUS")?).join("book.pdf")
    }
  };
  let mut doc = Document::open(&path, "")?;
  let handle = Handle::open(&path, "")?;
  let mut pages: Vec<_> =
    (0..24).map(|index| PageId(index * (doc.pages.len() - 1) / 23)).collect();
  pages.dedup();
  let mut margins = Margins::default();
  let lists = doc.lists();
  margins.update(&handle, &pages, &[], &lists, &AtomicBool::new(false))?;
  for index in 0..doc.pages.len() {
    let page = PageId(index);
    if !margins.contains(page) {
      margins.update(&handle, &[page], &[], &lists, &AtomicBool::new(false))?;
    }
    let crop = margins.get(page);
    eprintln!("page {}: {:?}", page.0 + 1, crop.0);
    let shown = crop.apply(doc.pages[page.0].bounds);
    for line in &doc.text(page)?.lines {
      for glyph in line.glyphs.iter().filter(|glyph| !glyph.ch.is_whitespace())
      {
        assert!(
          shown.contains(
            f32::midpoint(glyph.bounds.x0, glyph.bounds.x1),
            f32::midpoint(glyph.bounds.y0, glyph.bounds.y1),
          ),
          "page {} lost {:?}: {crop:?}",
          page.0 + 1,
          glyph.ch
        );
      }
    }
    let list = doc.display(page)?;
    check_pixels(&list, doc.pages[page.0].bounds, crop)?;
    let raw = list.to_text_page(
      mupdf::TextPageFlags::PRESERVE_IMAGES
        | mupdf::TextPageFlags::COLLECT_VECTORS,
    )?;
    for block in raw.blocks() {
      if block.r#type() == mupdf::text_page::TextBlockType::Image {
        let bounds = crate::document::rect(block.bounds());
        if let Some(bounds) = clip(bounds, doc.pages[page.0].bounds) {
          assert!(
            shown.contains(bounds.x0, bounds.y0)
              && shown.contains(bounds.x1, bounds.y1),
            "page {} lost image: {crop:?}",
            page.0 + 1
          );
        }
      }
    }
  }
  Ok(())
}

fn check_pixels(list: &DisplayList, bounds: Bounds, crop: Crop) -> Result<()> {
  let matrix = page_matrix(bounds, (WIDTH / bounds.width() * 1024.0) as u32, 0);
  let pixmap = list.to_pixmap(
    &crate::raster::matrix(&matrix),
    &Colorspace::device_gray(),
    false,
  )?;
  let masks = decoration::read(list, bounds, &matrix)?;
  let shown = transform(crop.apply(bounds), &matrix);
  let stride = pixmap.stride() as usize;
  for (y, row) in pixmap.samples().chunks(stride).enumerate() {
    for (x, value) in row[..pixmap.width() as usize].iter().enumerate() {
      let (x, y) = (x as f32 + pixmap.x() as f32, y as f32 + pixmap.y() as f32);
      assert!(
        *value >= INK
          || shown.contains(x, y)
          || masks.hidden(x, y)
          || masks.stamps.iter().any(|stamp| stamp.contains(x, y)),
        "lost ink at {x},{y}: {crop:?}"
      );
    }
  }
  Ok(())
}

#[test]
fn row_extents_match_pixels() {
  let rows: &[&[u8]] = &[
    &[],
    &[255],
    &[234],
    &[235],
    &[0, 255, 234],
    &[255, 0, 0, 255, 234, 255],
    &[0, 0, 0, 0, 0, 0],
  ];
  for row in rows {
    for origin in [-3.0, 0.0, 0.5] {
      for stamp in [None, Some((0, 1)), Some((1, 4)), Some((0, 99))] {
        let allowed = |x| stamp.is_none_or(|(a, b)| x < a || x > b);
        let actual = extent(row, 7.0, origin, allowed);
        let expected = row
          .iter()
          .enumerate()
          .filter(|(x, value)| **value < INK && allowed(*x))
          .map(|(x, _)| Bounds {
            x0: x as f32 + origin,
            y0: 7.0,
            x1: x as f32 + origin + 1.0,
            y1: 8.0,
          })
          .reduce(Bounds::union);
        assert_eq!(actual, expected, "{row:?}: {origin}: {stamp:?}");
      }
    }
  }
}
