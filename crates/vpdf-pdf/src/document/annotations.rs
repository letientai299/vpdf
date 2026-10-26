use super::{
  Annotation, Arc, Bounds, Document, Link, PageId, Result, Target, printable,
  quad_bounds, rect, uri,
};

impl Document {
  /// Annotation links, then addresses printed in the text. Cached:
  /// hover tests them per move.
  pub fn links(&mut self, page: PageId) -> Result<Arc<Vec<Link>>> {
    self.validate(page)?;
    if let Some(links) = self.links.get(&page) {
      return Ok(Arc::clone(links));
    }
    let mut links: Vec<_> = self
      .native
      .load_page(page.0 as i32)?
      .links()?
      .map(|link| {
        let target = link
          .dest
          .map_or_else(|| Target::Uri(uri(&link.uri)), super::types::target);
        Link::new(rect(link.bounds), target)
      })
      .collect();
    // Unreadable text must not hide annotation links.
    if let Ok(text) = self.text(page) {
      links = crate::wraps::join(links, &text, page);
      links.extend(crate::autolink::find(&text, &links));
    }
    let links = Arc::new(links);
    let weight = links.capacity() * size_of::<Link>()
      + links.iter().map(Link::weight).sum::<usize>();
    self.links.insert(page, Arc::clone(&links), weight.max(1));
    Ok(links)
  }

  /// Non-link annotations, cached for hover.
  pub fn annotations(&mut self, page: PageId) -> Result<Arc<Vec<Annotation>>> {
    self.validate(page)?;
    if let Some(found) = self.annotations.get(&page) {
      return Ok(Arc::clone(found));
    }
    let found = Arc::new(self.read_annotations(page)?);
    let heap = found.iter().map(|note| {
      note.kind.capacity()
        + note.areas.capacity() * size_of::<Bounds>()
        + note.contents.capacity()
        + note.author.capacity()
    });
    let weight =
      found.capacity() * size_of::<Annotation>() + heap.sum::<usize>();
    // Heavier pages parse again per use.
    self.annotations.insert(page, Arc::clone(&found), weight.max(1));
    Ok(found)
  }

  fn read_annotations(&self, page: PageId) -> Result<Vec<Annotation>> {
    use mupdf::pdf::PdfAnnotationType as Kind;
    // Most pages list none; the lookup skips loading those pages.
    let listed = self.native.find_page(page.0 as i32)?.get_dict("Annots")?;
    if listed.map_or(Ok(0), |list| list.len())? == 0 {
      return Ok(Vec::new());
    }
    let native = self.native.load_pdf_page(page.0 as i32)?;
    let mut found = Vec::new();
    for annotation in native.annotations() {
      let kind = annotation.r#type()?;
      if matches!(kind, Kind::Link | Kind::Popup | Kind::Widget) {
        continue;
      }
      let quads = annotation.quad_points().unwrap_or_default();
      let areas = if quads.is_empty() {
        vec![rect(annotation.rect()?)]
      } else {
        quads.iter().map(quad_bounds).collect()
      };
      let clean = |text: Option<&str>| {
        printable(text.unwrap_or_default())
          .collect::<String>()
          .trim()
          .to_owned()
      };
      let contents = clean(annotation.contents()?);
      let author = clean(annotation.author()?);
      let kind =
        if kind == Kind::Text { "Note".into() } else { format!("{kind:?}") };
      let color = annotation.color()?.map(rgb);
      found.push(Annotation { kind, areas, contents, author, color });
    }
    Ok(found)
  }
}

/// A color as 8-bit RGB; CMYK converts without a profile.
fn rgb(color: mupdf::color::AnnotationColor) -> [u8; 3] {
  use mupdf::color::AnnotationColor as C;
  let [red, green, blue] = match color {
    C::Gray(gray) => [gray; 3],
    C::Rgb { red, green, blue } => [red, green, blue],
    C::Cmyk { cyan, magenta, yellow, key } => {
      [cyan, magenta, yellow].map(|ink| (1.0 - ink) * (1.0 - key))
    }
  };
  [red, green, blue]
    .map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8)
}

#[cfg(test)]
mod tests {
  use mupdf::color::AnnotationColor as C;

  #[test]
  fn colors_convert_to_rgb() {
    let cases = [
      (C::Gray(0.5), [128, 128, 128]),
      (C::Rgb { red: 1.0, green: 0.92, blue: 0.23 }, [255, 235, 59]),
      (C::Rgb { red: 2.0, green: -1.0, blue: 0.0 }, [255, 0, 0]),
      (
        C::Cmyk { cyan: 0.0, magenta: 0.0, yellow: 1.0, key: 0.0 },
        [255, 255, 0],
      ),
      (
        C::Cmyk { cyan: 0.0, magenta: 0.0, yellow: 0.0, key: 0.5 },
        [128, 128, 128],
      ),
    ];
    for (color, want) in cases {
      assert_eq!(super::rgb(color), want, "{want:?}");
    }
  }
}
