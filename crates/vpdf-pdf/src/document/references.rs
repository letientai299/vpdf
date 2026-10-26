use super::{Bounds, Document, PageId, Result, Target, pdf_rect, rect};
use anyhow::{Context, ensure};
use mupdf::pdf::PdfObject;
use std::fmt::Write;

impl Document {
  pub fn named_destination(&self, name: &[u8]) -> Result<Target> {
    let page = self.destination_page(self.named_value(name)?)?;
    let mut uri = String::from("#nameddest=");
    for byte in name {
      write!(uri, "%{byte:02X}")?;
    }
    let dest = self
      .native
      .resolve_link(&uri)?
      .context("Named destination unresolved")?;
    ensure!(
      dest.loc.page_number as usize == page.0,
      "Named destination page mismatch"
    );
    Ok(super::types::target(dest))
  }

  fn named_value(&self, name: &[u8]) -> Result<PdfObject> {
    let root = self
      .native
      .trailer()?
      .get_dict("Root")?
      .context("PDF catalog missing")?;
    if let Some(legacy) = root.get_dict("Dests")? {
      for entry in legacy.dict_iter()? {
        let (key, value) = entry?;
        if key.as_name()? == name {
          return Ok(value);
        }
      }
      anyhow::bail!("Named destination missing");
    }
    if let Some(names) = root.get_dict("Names")?
      && let Some(tree) = names.get_dict("Dests")?
    {
      return find_name(tree, |key| text_name(key, name))?
        .context("Named destination missing");
    }
    anyhow::bail!("Named destination missing")
  }

  fn destination_page(&self, mut destination: PdfObject) -> Result<PageId> {
    for _ in 0..10 {
      if destination.is_array()? {
        let object =
          destination.get_array(0)?.context("Destination page missing")?;
        // MuPDF clamps unresolved destination pages.
        let number = if object.is_int()? {
          object.as_int()?
        } else {
          self.native.lookup_page_number(&object)?
        };
        ensure!(number >= 0, "Destination page unresolved");
        let page = PageId(number as usize);
        self.validate(page)?;
        return Ok(page);
      }
      destination = if destination.is_dict()? {
        destination.get_dict("D")?.context("Destination value missing")?
      } else if destination.is_name()? {
        self.named_value(&destination.as_name()?)?
      } else if destination.is_string()? {
        self.named_value(destination.as_string()?.as_bytes())?
      } else {
        anyhow::bail!("Destination value invalid")
      };
    }
    anyhow::bail!("Destination traversal limit reached")
  }

  pub fn structure_target(&self, id: &[u8]) -> Result<Target> {
    let root = self
      .native
      .trailer()?
      .get_dict("Root")?
      .context("PDF catalog missing")?;
    let tree = root
      .get_dict("StructTreeRoot")?
      .context("Structure tree missing")?
      .get_dict("IDTree")?
      .context("Structure ID tree missing")?;
    let element = find_name(tree, |key| Ok(key.as_bytes()? == id))?
      .context("Structure ID missing")?;
    let mut pending = vec![element];
    for _ in 0..4096 {
      let Some(element) = pending.pop() else {
        anyhow::bail!("Structure page missing")
      };
      if element.is_array()? {
        for index in (0..element.len()?).rev() {
          pending.extend(element.get_array(index as i32)?);
        }
      } else if element.is_dict()? {
        if let Some(page) = element.get_dict("Pg")? {
          let number = self.native.lookup_page_number(&page)?;
          ensure!(number >= 0, "Structure page unresolved");
          let page = PageId(number as usize);
          self.validate(page)?;
          return Ok(Target::Page(page));
        }
        if let Some(kids) = element.get_dict("K")? {
          pending.push(kids);
        }
      }
    }
    anyhow::bail!("Structure traversal limit reached")
  }

  pub fn comment_bounds(&self, page: PageId, name: &[u8]) -> Result<Bounds> {
    self.validate(page)?;
    let object = self.native.find_page(page.0 as i32)?;
    let annotations =
      object.get_dict("Annots")?.context("Annotation name missing")?;
    for annotation in annotations.array_iter()? {
      let annotation = annotation?;
      if let Some(id) = annotation.get_dict("NM")?
        && text_name(&id, name)?
      {
        let raw = annotation
          .get_dict("Rect")?
          .context("Annotation rectangle missing")?;
        let bounds = pdf_rect(&raw)?.context("Annotation rectangle invalid")?;
        let bounds =
          mupdf::Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y1)
            .transform(&object.page_ctm()?);
        let bounds =
          vpdf_model::geometry::clip(rect(bounds), self.pages[page.0].bounds)
            .context("Annotation outside page")?;
        return self.visible_rectangle(page, bounds);
      }
    }
    anyhow::bail!("Annotation name missing")
  }

  pub fn highlight_bounds(
    &self,
    page: PageId,
    values: [f32; 4],
  ) -> Result<Bounds> {
    self.validate(page)?;
    let bounds = self.pages[page.0].bounds;
    let [left, right, top, bottom] = values;
    self.visible_rectangle(
      page,
      Bounds {
        x0: bounds.x0 + left,
        x1: bounds.x0 + right,
        y0: bounds.y0 + top,
        y1: bounds.y0 + bottom,
      },
    )
  }

  fn visible_rectangle(&self, page: PageId, area: Bounds) -> Result<Bounds> {
    let bounds = self.pages[page.0].bounds;
    ensure!(
      [area.x0, area.x1, area.y0, area.y1].iter().all(|v| v.is_finite())
        && area.x0 < area.x1
        && area.y0 < area.y1
        && area.x0 >= bounds.x0
        && area.x1 <= bounds.x1
        && area.y0 >= bounds.y0
        && area.y1 <= bounds.y1,
      "Rectangle outside page"
    );
    Ok(area)
  }
}

fn text_name(object: &PdfObject, name: &[u8]) -> Result<bool> {
  Ok(object.as_bytes()? == name || object.as_string()?.as_bytes() == name)
}

fn find_name(
  tree: PdfObject,
  matches: impl Fn(&PdfObject) -> Result<bool>,
) -> Result<Option<PdfObject>> {
  let mut pending = vec![tree];
  for _ in 0..4096 {
    let Some(node) = pending.pop() else { return Ok(None) };
    if let Some(names) = node.get_dict("Names")? {
      let mut entries = names.array_iter()?;
      while let Some(key) = entries.next() {
        let value = entries.next().context("Name tree value missing")??;
        if matches(&key?)? {
          return Ok(Some(value));
        }
      }
    }
    if let Some(kids) = node.get_dict("Kids")? {
      for child in kids.array_iter()? {
        pending.push(child?);
      }
    }
  }
  anyhow::bail!("Name tree traversal limit reached")
}

#[cfg(test)]
mod tests;
