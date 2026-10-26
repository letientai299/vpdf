use crate::Result;
use crate::cache::Cache;
use crate::text::{Bounds, Glyph, Line, PageId, Text, printable};
use anyhow::ensure;
use mupdf::pdf::PdfDocument;
use mupdf::{DisplayList, TextPageFlags};
use std::cell::OnceCell;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub const GLYPH_LIMIT: usize = 40_000;
pub const MIB: usize = 1024 * 1024;

/// Display lists by page. Handles of one file share them, so a page is
/// interpreted once for both its text and its tiles; `MuPDF` renders a
/// list on any thread while its document stays on one.
pub type Lists = Mutex<Cache<PageId, Arc<DisplayList>>>;

/// Pages whose display lists stay cached: the visible pages and the
/// ten-page neighborhood and nearby render tiles.
const LISTS: usize = 16;

pub struct Document {
  pub native: PdfDocument,
  pub pages: Vec<PageInfo>,
  pub outline: Vec<Heading>,
  /// Info dictionary title; empty when absent.
  pub title: String,
  /// The outline was inferred from text layout, not authored.
  pub inferred: bool,
  /// Cached page margins for `zc`.
  pub(crate) margins: Option<Arc<crate::crop::Margins>>,
  texts: Cache<PageId, Arc<Text>>,
  lists: Arc<Lists>,
  ocr: Cache<PageId, Arc<Text>>,
  figures: Cache<PageId, Arc<Vec<crate::figures::Figure>>>,
  links: Cache<PageId, Arc<Vec<Link>>>,
  annotations: Cache<PageId, Arc<Vec<Annotation>>>,
}

/// The document is encrypted and the password did not open it.
#[derive(Debug)]
pub struct PasswordRequired;

impl std::fmt::Display for PasswordRequired {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str("PDF password required")
  }
}

impl std::error::Error for PasswordRequired {}

/// A worker's handle of a file: the native document and its page count,
/// opened without the page walk of `Document::open`.
pub struct Handle {
  pub native: PdfDocument,
  pub pages: usize,
  pub(crate) cacheable: bool,
}

impl Handle {
  pub fn open(path: &Path, password: &str) -> Result<Self> {
    crate::fonts::install();
    let mut native = PdfDocument::open(path)?;
    if native.needs_password()? && !native.authenticate(password)? {
      return Err(PasswordRequired.into());
    }
    let pages = usize::try_from(native.page_count()?)?;
    ensure!(pages > 0, "PDF contains no pages");
    let cacheable = crate::content_cache::cacheable(path, password);
    Ok(Self { native, pages, cacheable })
  }

  fn validate(&self, page: PageId) -> Result<()> {
    ensure!(page.0 < self.pages, "Page outside document");
    Ok(())
  }

  /// The bounds `Document::open` reads for `page`.
  pub fn bounds(&self, page: PageId) -> Result<Bounds> {
    self.validate(page)?;
    bounds(&self.native, page.0 as i32)
  }

  /// `page`'s display list from `lists`, interpreted on a miss.
  pub fn display(
    &self,
    page: PageId,
    lists: &Lists,
  ) -> Result<Arc<DisplayList>> {
    self.validate(page)?;
    display(&self.native, lists, page)
  }

  /// The authored outline. Resolving its destinations can load most of a
  /// large book's objects, which a worker's handle releases when dropped.
  pub fn outline(&self) -> Result<Vec<Heading>> {
    outline(&self.native)
  }

  pub fn metadata(&self) -> Vec<(String, String)> {
    metadata(&self.native)
  }

  /// The page contents' text, without annotations, uncached: workers
  /// read a page once.
  pub fn text(&self, page: PageId) -> Result<Text> {
    self.validate(page)?;
    extract(&self.native.load_page(page.0 as i32)?)
  }

  /// Like [`Self::text`], with font facts per line.
  pub fn styled_text(&self, page: PageId) -> Result<(Text, Vec<LineStyle>)> {
    self.validate(page)?;
    extract_styled(&self.native.load_page(page.0 as i32)?)
  }
}

fn display(
  native: &PdfDocument,
  lists: &Lists,
  page: PageId,
) -> Result<Arc<DisplayList>> {
  let cached =
    lists.lock().ok().and_then(|mut lists| lists.get(&page).map(Arc::clone));
  if let Some(list) = cached {
    return Ok(list);
  }
  let list = Arc::new(native.load_page(page.0 as i32)?.to_display_list(true)?);
  if let Ok(mut lists) = lists.lock() {
    lists.insert(page, Arc::clone(&list), 1);
  }
  Ok(list)
}

/// A page's bounds, from its dictionary when that reads, else from the
/// loaded page.
fn bounds(native: &PdfDocument, index: i32) -> Result<Bounds> {
  let bounds = match page_bounds(native, index) {
    Ok(bounds) => bounds,
    Err(_) => rect(native.load_page(index)?.bounds()?),
  };
  ensure!(
    bounds.width() > 0.0
      && bounds.height() > 0.0
      && bounds.width().is_finite()
      && bounds.height().is_finite(),
    "PDF page has invalid dimensions"
  );
  Ok(bounds)
}

/// A page's crop box bound as `fz_bound_page` computes it, read from the
/// page dictionary without loading the page, whose annotations can cost
/// more than the rest of opening a document. Mirrors
/// `pdf_page_obj_transform_box` in `source/pdf/pdf-page.c`.
fn page_bounds(native: &PdfDocument, index: i32) -> Result<Bounds> {
  let page = native.find_page(index)?;
  let media = match page.get_dict_inheritable("MediaBox")? {
    Some(media) => pdf_rect(&media)?,
    None => None,
  };
  let used = match (media, page.get_dict_inheritable("CropBox")?) {
    (media, None) => media,
    (Some(media), Some(crop)) => pdf_rect(&crop)?.map(|crop| Bounds {
      x0: media.x0.max(crop.x0),
      y0: media.y0.max(crop.y0),
      x1: media.x1.min(crop.x1),
      y1: media.y1.min(crop.y1),
    }),
    (None, Some(_)) => None,
  };
  let used = used.filter(|r| r.x0 < r.x1 && r.y0 < r.y1).unwrap_or(Bounds {
    x0: 0.0,
    y0: 0.0,
    x1: 612.0,
    y1: 792.0,
  });
  let used = if used.width() < 1.0 || used.height() < 1.0 {
    Bounds { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 }
  } else {
    used
  };
  let rect = mupdf::Rect::new(used.x0, used.y0, used.x1, used.y1);
  Ok(self::rect(rect.transform(&page.page_ctm()?)))
}

/// A rectangle array as `pdf_to_rect` reads it; none when not an array.
fn pdf_rect(object: &mupdf::pdf::PdfObject) -> Result<Option<Bounds>> {
  if !object.is_array()? {
    return Ok(None);
  }
  let mut values = [0.0_f32; 4];
  for (index, value) in values.iter_mut().enumerate() {
    if let Some(item) = object.get_array(index as i32)? {
      *value = item.as_float()?;
    }
  }
  let [a, b, c, d] = values;
  Ok(Some(Bounds { x0: a.min(c), y0: b.min(d), x1: a.max(c), y1: b.max(d) }))
}

/// Drops controls from URLs.
fn uri(raw: &str) -> String {
  raw.chars().filter(|ch| !ch.is_control()).collect()
}

fn quad_bounds(quad: &mupdf::Quad) -> Bounds {
  Bounds::enclosing(&[quad.ul, quad.ur, quad.ll, quad.lr].map(|p| (p.x, p.y)))
}
impl Document {
  pub fn open(path: &Path, password: &str) -> Result<Self> {
    let handle = Handle::open(path, password)?;
    let bounds = (0..handle.pages)
      .map(|index| bounds(&handle.native, index as i32))
      .collect::<Result<_>>()?;
    Ok(Self::from_handle(handle, bounds))
  }

  /// Opens `path` with page bounds another handle of it read, skipping
  /// the page walk. Checks only the page count, not the bounds.
  pub fn open_known(
    path: &Path,
    password: &str,
    bounds: Vec<Bounds>,
  ) -> Result<Self> {
    let handle = Handle::open(path, password)?;
    ensure!(handle.pages == bounds.len(), "PDF page count changed");
    Ok(Self::from_handle(handle, bounds))
  }

  fn from_handle(handle: Handle, bounds: Vec<Bounds>) -> Self {
    let Handle { native, .. } = handle;
    let pages = bounds
      .into_iter()
      .map(|bounds| PageInfo { bounds, label: OnceCell::new() })
      .collect();
    let title =
      native.metadata(mupdf::MetadataName::Title).map_or_default(|title| {
        printable(&title).collect::<String>().trim().to_owned()
      });
    Self {
      native,
      pages,
      outline: Vec::new(),
      title,
      inferred: false,
      margins: None,
      texts: Cache::new(32 * MIB),
      lists: Arc::new(Mutex::new(Cache::new(LISTS))),
      ocr: Cache::new(32 * MIB),
      figures: Cache::new(4 * MIB),
      links: Cache::new(MIB),
      annotations: Cache::new(MIB),
    }
  }

  pub fn text(&mut self, page: PageId) -> Result<Arc<Text>> {
    self.validate(page)?;
    if let Some(text) = self.ocr.get(&page).or_else(|| self.texts.get(&page)) {
      return Ok(Arc::clone(text));
    }
    // Motions wait on this text; background scans wait for it.
    let _urgent = crate::scheduling::Urgent::new();
    let list = self.display(page)?;
    let text = Arc::new(extract_list(&list)?);
    self.texts.insert(page, Arc::clone(&text), text.weight());
    Ok(text)
  }

  pub fn figures(
    &mut self,
    page: PageId,
  ) -> Result<Arc<Vec<crate::figures::Figure>>> {
    self.validate(page)?;
    if let Some(figures) = self.figures.get(&page) {
      return Ok(Arc::clone(figures));
    }
    let list = self.display(page)?;
    let text = self.text(page)?;
    let figures = Arc::new(crate::figures::detect(
      &list,
      &text,
      self.pages[page.0].bounds,
    )?);
    let weight = figures.capacity() * size_of::<crate::figures::Figure>()
      + figures.iter().map(|figure| figure.caption.capacity()).sum::<usize>();
    self.figures.insert(page, Arc::clone(&figures), weight.max(1));
    Ok(figures)
  }

  /// Page margins a crop scan measured.
  pub fn margins(&self) -> Option<&Arc<crate::crop::Margins>> {
    self.margins.as_ref()
  }

  /// Takes the margins of another document of the same file.
  pub fn share_margins(&mut self, other: &Self) {
    self.margins.clone_from(&other.margins);
  }

  /// The display list cache, for another handle of this file.
  pub fn lists(&self) -> Arc<Lists> {
    Arc::clone(&self.lists)
  }

  /// Drops caches except OCR text.
  pub fn trim(&mut self) {
    self.texts.clear();
    self.figures.clear();
    self.links.clear();
    self.annotations.clear();
    if let Ok(mut lists) = self.lists.lock() {
      lists.clear();
    }
  }

  pub fn has_ocr(&self, page: PageId) -> bool {
    self.ocr.contains(&page)
  }

  pub fn seed(&mut self, page: PageId, text: Arc<Text>) {
    self.figures.remove(&page);
    self.links.remove(&page);
    let weight = text.weight();
    self.ocr.insert(page, text, weight);
  }

  pub fn display(&mut self, page: PageId) -> Result<Arc<DisplayList>> {
    self.validate(page)?;
    display(&self.native, &self.lists, page)
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

pub fn rect(rect: mupdf::Rect) -> Bounds {
  Bounds { x0: rect.x0, y0: rect.y0, x1: rect.x1, y1: rect.y1 }
}

pub use extraction::{TEXT_FLAGS, extract, extract_list, extract_styled};
#[cfg(test)]
use metadata::date;
use metadata::{metadata, outline};
pub use style::LineStyle;
pub use types::PageInfo;
pub(crate) use types::target as link_target;
pub use vpdf_model::document::{Annotation, Heading, Link, Target};

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn dates_read_plainly() {
    let cases = [
      ("D:20230802120000+02'00'", "2023-08-02 12:00:00 +02:00"),
      ("D:20230802000000Z", "2023-08-02 00:00:00 Z"),
      ("D:20230802", "2023-08-02"),
      ("yesterday", "yesterday"),
    ];
    for (raw, expected) in cases {
      assert_eq!(date(raw), expected, "{raw}");
    }
  }
}

mod annotations;
mod extraction;
mod metadata;
mod references;
mod style;
mod types;
