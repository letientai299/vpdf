use crate::Result;
use crate::cache::Cache;
use crate::text::{Bounds, Glyph, Line, PageId, Text};
use anyhow::{bail, ensure};
use mupdf::pdf::PdfDocument;
use mupdf::{DisplayList, TextPageFlags};
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;

pub const GLYPH_LIMIT: usize = 40_000;
pub const MIB: usize = 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct PageInfo {
  pub bounds: Bounds,
  pub label: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Link {
  pub bounds: Bounds,
  pub target: Target,
}

#[derive(Clone, Debug, Serialize)]
pub enum Target {
  Page(PageId),
  Point { page: PageId, x: f32, y: f32 },
  Uri(String),
}

impl From<mupdf::link::LinkDestination> for Target {
  fn from(dest: mupdf::link::LinkDestination) -> Self {
    use mupdf::DestinationKind as D;
    let page = PageId(dest.loc.page_number as usize);
    let (x, y) = match dest.kind {
      D::XYZ { left, top, .. } => (left, top),
      D::FitH { top } | D::FitBH { top } => (None, top),
      D::FitV { left } | D::FitBV { left } => (left, None),
      _ => return Self::Page(page),
    };
    if x.is_none() && y.is_none() {
      return Self::Page(page);
    }
    Self::Point {
      page,
      x: x.filter(|v| v.is_finite()).unwrap_or(0.0),
      y: y.filter(|v| v.is_finite()).unwrap_or(0.0),
    }
  }
}

/// A reader-visible annotation: markup, note, or shape.
#[derive(Clone, Debug, Serialize)]
pub struct Annotation {
  pub kind: String,
  /// Marked areas; markup has one per quad.
  pub areas: Vec<Bounds>,
  pub contents: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Heading {
  pub title: String,
  pub depth: usize,
  pub target: Target,
}

pub struct Document {
  pub native: PdfDocument,
  pub pages: Vec<PageInfo>,
  pub outline: Vec<Heading>,
  /// Info dictionary title; empty when absent.
  pub title: String,
  texts: Cache<PageId, Arc<Text>>,
  lists: Cache<PageId, Arc<DisplayList>>,
  ocr: Cache<PageId, Arc<Text>>,
}

impl Document {
  pub fn open(path: &Path, password: &str) -> Result<Self> {
    let mut native = PdfDocument::open(path)?;
    if native.needs_password()? && !native.authenticate(password)? {
      bail!("PDF password required");
    }
    let count = native.page_count()?;
    ensure!(count > 0, "PDF contains no pages");
    let mut pages = Vec::with_capacity(count as usize);
    for index in 0..count {
      let bounds: Bounds = native.load_page(index)?.bounds()?.into();
      ensure!(
        bounds.width() > 0.0
          && bounds.height() > 0.0
          && bounds.width().is_finite()
          && bounds.height().is_finite(),
        "PDF page has invalid dimensions"
      );
      pages.push(PageInfo {
        bounds,
        label: native
          .page_label(index as usize)
          .ok()
          .filter(|label| !label.is_empty())
          .unwrap_or_else(|| (index + 1).to_string()),
      });
    }
    let mut outline = Vec::new();
    flatten(&native.outlines()?, 0, &mut outline);
    let title = native
      .metadata(mupdf::MetadataName::Title)
      .map(|title| title.trim().to_owned())
      .unwrap_or_default();
    Ok(Self {
      native,
      pages,
      outline,
      title,
      texts: Cache::new(32 * MIB),
      lists: Cache::new(4),
      ocr: Cache::new(32 * MIB),
    })
  }

  pub fn text(&mut self, page: PageId) -> Result<Arc<Text>> {
    self.validate(page)?;
    if let Some(text) = self.ocr.get(&page).or_else(|| self.texts.get(&page)) {
      return Ok(Arc::clone(text));
    }
    let native = self.native.load_page(page.0 as i32)?;
    let text = Arc::new(extract(&native)?);
    self.texts.insert(page, Arc::clone(&text), text.weight());
    Ok(text)
  }

  pub fn seed(&mut self, page: PageId, text: Arc<Text>) {
    let weight = text.weight();
    self.ocr.insert(page, text, weight);
  }

  pub fn display(&mut self, page: PageId) -> Result<Arc<DisplayList>> {
    self.validate(page)?;
    if let Some(list) = self.lists.get(&page) {
      return Ok(Arc::clone(list));
    }
    let page_data = self.native.load_page(page.0 as i32)?;
    let list = Arc::new(page_data.to_display_list(true)?);
    self.lists.insert(page, Arc::clone(&list), 1);
    Ok(list)
  }

  pub fn links(&self, page: PageId) -> Result<Vec<Link>> {
    self.validate(page)?;
    Ok(
      self
        .native
        .load_page(page.0 as i32)?
        .links()?
        .map(|link| Link {
          bounds: link.bounds.into(),
          target: link.dest.map_or_else(|| Target::Uri(link.uri), Target::from),
        })
        .collect(),
    )
  }

  /// Annotations other than links, popups, and form widgets.
  pub fn annotations(&self, page: PageId) -> Result<Vec<Annotation>> {
    use mupdf::pdf::PdfAnnotationType as Kind;
    self.validate(page)?;
    let native = self.native.load_pdf_page(page.0 as i32)?;
    let mut found = Vec::new();
    for annotation in native.annotations() {
      let kind = annotation.r#type()?;
      if matches!(kind, Kind::Link | Kind::Popup | Kind::Widget) {
        continue;
      }
      let quads = annotation.quad_points().unwrap_or_default();
      let areas = if quads.is_empty() {
        vec![annotation.rect()?.into()]
      } else {
        quads.iter().map(quad_bounds).collect()
      };
      let contents =
        annotation.contents()?.unwrap_or_default().trim().to_owned();
      let kind =
        if kind == Kind::Text { "Note".into() } else { format!("{kind:?}") };
      found.push(Annotation { kind, areas, contents });
    }
    Ok(found)
  }

  pub fn page_number(&self, label: &str) -> Option<PageId> {
    if let Ok(number) = label.parse::<usize>() {
      return (number > 0 && number <= self.pages.len())
        .then(|| PageId(number - 1));
    }
    self.pages.iter().position(|p| p.label == label).map(PageId)
  }

  pub fn validate(&self, page: PageId) -> Result<()> {
    ensure!(page.0 < self.pages.len(), "Page outside document");
    Ok(())
  }

  pub fn seeds(&self) -> Vec<(PageId, Arc<Text>)> {
    self.ocr.entries().map(|(page, text)| (*page, Arc::clone(text))).collect()
  }

  pub fn cache_bytes(&self) -> usize {
    self.texts.used() + self.ocr.used()
  }
}

fn quad_bounds(quad: &mupdf::Quad) -> Bounds {
  Bounds::enclosing(&[quad.ul, quad.ur, quad.ll, quad.lr].map(|p| (p.x, p.y)))
}

fn flatten(items: &[mupdf::Outline], depth: usize, out: &mut Vec<Heading>) {
  if depth > 64 || out.len() >= 10_000 {
    return;
  }
  for item in items {
    if out.len() >= 10_000 {
      break;
    }
    let target = item
      .dest
      .map(Target::from)
      .or_else(|| item.uri.as_ref().map(|u| Target::Uri(u.clone())));
    if let Some(target) = target {
      out.push(Heading { title: item.title.clone(), depth, target });
    }
    flatten(&item.down, depth + 1, out);
  }
}

pub fn extract(page: &mupdf::Page) -> Result<Text> {
  let flags = TextPageFlags::PRESERVE_LIGATURES
    | TextPageFlags::PRESERVE_WHITESPACE
    | TextPageFlags::CLIP
    | TextPageFlags::USE_CID_FOR_UNKNOWN_UNICODE;
  let raw = page.to_text_page(flags)?;
  let mut text = Text::default();
  let mut count = 0;
  for block in raw.blocks() {
    let mut paragraph = true;
    for line in block.lines() {
      let mut row = Line::default();
      for glyph in line.chars() {
        let Some(ch) = glyph.char() else { continue };
        if ch.is_control() {
          continue;
        }
        if count >= GLYPH_LIMIT {
          text.truncated = true;
          break;
        }
        let q = glyph.quad();
        let bounds = Bounds {
          x0: q.ul.x.min(q.ll.x).min(q.ur.x).min(q.lr.x),
          y0: q.ul.y.min(q.ll.y).min(q.ur.y).min(q.lr.y),
          x1: q.ul.x.max(q.ll.x).max(q.ur.x).max(q.lr.x),
          y1: q.ul.y.max(q.ll.y).max(q.ur.y).max(q.lr.y),
        };
        row.glyphs.push(Glyph { ch, bounds });
        count += 1;
      }
      if row.glyphs.iter().any(|g| !g.ch.is_whitespace()) {
        row.paragraph = paragraph;
        paragraph = false;
        text.lines.push(row);
      }
      if text.truncated {
        return Ok(text);
      }
    }
  }
  Ok(text)
}
