use crate::Result;
use crate::cache::Cache;
use crate::layout::Crop;
use crate::text::{Bounds, Glyph, Line, PageId, Text, printable};
use anyhow::ensure;
use mupdf::pdf::PdfDocument;
use mupdf::{DisplayList, TextPageFlags};
use std::cell::OnceCell;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub const GLYPH_LIMIT: usize = 40_000;
pub const MIB: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct PageInfo {
  pub bounds: Bounds,
  /// Read on first use through `Document::label`: reading every label
  /// can cost more than the rest of opening a long manual.
  pub label: OnceCell<String>,
}

#[derive(Clone, Debug)]
pub struct Link {
  first: Bounds,
  /// Lines or columns it wraps onto.
  rest: Vec<Bounds>,
  pub target: Target,
}

impl Link {
  pub fn new(first: Bounds, target: Target) -> Self {
    Self { first, rest: Vec::new(), target }
  }

  pub fn first(&self) -> Bounds {
    self.first
  }

  pub fn last(&self) -> Bounds {
    self.rest.last().copied().unwrap_or(self.first)
  }

  /// Every rectangle, in reading order.
  pub fn boxes(&self) -> impl Iterator<Item = Bounds> + '_ {
    std::iter::once(self.first).chain(self.rest.iter().copied())
  }

  pub fn contains(&self, x: f32, y: f32) -> bool {
    self.boxes().any(|bounds| bounds.contains(x, y))
  }

  /// Its text; wrapped parts join unspaced.
  pub fn anchor(&self, text: &Text, page: PageId) -> String {
    self.boxes().map(|bounds| text.in_areas(page, &[bounds]).1).collect()
  }

  /// Takes `part`'s boxes after its own.
  pub(crate) fn append(&mut self, part: Self) {
    self.rest.push(part.first);
    self.rest.extend(part.rest);
  }

  /// Takes `part`'s boxes before its own.
  pub(crate) fn prepend(&mut self, part: Self) {
    let own = std::mem::replace(&mut self.rest, part.rest);
    self.rest.push(std::mem::replace(&mut self.first, part.first));
    self.rest.extend(own);
  }

  /// Adds a box after its own.
  pub(crate) fn push(&mut self, bounds: Bounds) {
    self.rest.push(bounds);
  }

  /// Heap bytes beyond the struct.
  pub(crate) fn weight(&self) -> usize {
    let uri = match &self.target {
      Target::Uri(uri) => uri.capacity(),
      Target::Page(_) | Target::Point { .. } => 0,
    };
    self.rest.capacity() * size_of::<Bounds>() + uri
  }
}

#[derive(Clone, Debug, PartialEq)]
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
#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
  pub kind: String,
  /// Marked areas; markup has one per quad.
  pub areas: Vec<Bounds>,
  pub contents: String,
  pub author: String,
}

#[derive(Clone, Debug)]
pub struct Heading {
  pub title: String,
  pub depth: usize,
  pub target: Target,
}

/// Display lists by page. Handles of one file share them, so a page is
/// interpreted once for both its text and its tiles; `MuPDF` renders a
/// list on any thread while its document stays on one.
pub type Lists = Mutex<Cache<PageId, Arc<DisplayList>>>;

/// Pages whose display lists stay cached: the visible pages and the
/// next screen in either direction.
const LISTS: usize = 8;

pub struct Document {
  pub native: PdfDocument,
  pub pages: Vec<PageInfo>,
  pub outline: Vec<Heading>,
  /// Info dictionary title; empty when absent.
  pub title: String,
  /// The outline was inferred from text layout, not authored.
  pub inferred: bool,
  /// Margins `zc` found; sampling again evicts the visible lists.
  pub margins: Option<Crop>,
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
    Ok(Self { native, pages })
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
}

impl Document {
  pub fn open(path: &Path, password: &str) -> Result<Self> {
    let Handle { native, pages: count } = Handle::open(path, password)?;
    let mut pages = Vec::with_capacity(count);
    for index in 0..count {
      let bounds = bounds(&native, index as i32)?;
      pages.push(PageInfo { bounds, label: OnceCell::new() });
    }
    let title =
      native.metadata(mupdf::MetadataName::Title).map_or_default(|title| {
        printable(&title).collect::<String>().trim().to_owned()
      });
    Ok(Self {
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
    })
  }

  pub fn text(&mut self, page: PageId) -> Result<Arc<Text>> {
    self.validate(page)?;
    if let Some(text) = self.ocr.get(&page).or_else(|| self.texts.get(&page)) {
      return Ok(Arc::clone(text));
    }
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

  /// Reads the authored outline into `outline`; readers load it on a
  /// worker instead.
  pub fn load_outline(&mut self) -> Result<()> {
    self.outline = outline(&self.native)?;
    Ok(())
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
        let target =
          link.dest.map_or_else(|| Target::Uri(uri(&link.uri)), Target::from);
        Link::new(link.bounds.into(), target)
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

  pub fn metadata(&self) -> Vec<(String, String)> {
    metadata(&self.native)
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
        vec![annotation.rect()?.into()]
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
      found.push(Annotation { kind, areas, contents, author });
    }
    Ok(found)
  }

  /// `page`'s authored label, printable, else its number; none past the
  /// last page.
  pub fn get_label(&self, page: PageId) -> Option<&str> {
    let label = &self.pages.get(page.0)?.label;
    Some(label.get_or_init(|| {
      let authored = self.native.page_label(page.0).ok();
      authored
        .map(|label| printable(&label).collect::<String>())
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| (page.0 + 1).to_string())
    }))
  }

  /// # Panics
  ///
  /// When `page` is past the last page.
  pub fn label(&self, page: PageId) -> &str {
    self.get_label(page).expect("page inside document")
  }

  /// Numbers name physical pages; anything else the first page with
  /// that label, which reads labels up to it.
  pub fn page_number(&self, label: &str) -> Option<PageId> {
    if let Ok(number) = label.parse::<usize>() {
      return (number > 0 && number <= self.pages.len())
        .then(|| PageId(number - 1));
    }
    (0..self.pages.len()).map(PageId).find(|page| self.label(*page) == label)
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
    Err(_) => native.load_page(index)?.bounds()?.into(),
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
  Ok(rect.transform(&page.page_ctm()?).into())
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

/// Format, encryption, and every Info dictionary entry, standard keys
/// first; dates read as `YYYY-MM-DD HH:MM:SS`.
fn metadata(native: &PdfDocument) -> Vec<(String, String)> {
  use mupdf::MetadataName as Name;
  const STANDARD: [&str; 8] = [
    "Title",
    "Author",
    "Subject",
    "Keywords",
    "Creator",
    "Producer",
    "CreationDate",
    "ModDate",
  ];
  let mut fields = Vec::new();
  for (key, name) in
    [("Format", Name::Format), ("Encryption", Name::Encryption)]
  {
    if let Ok(value) = native.metadata(name) {
      fields.push((key.to_owned(), value));
    }
  }
  let mut info = Vec::new();
  let dictionary =
    native.trailer().ok().and_then(|t| t.get_dict("Info").ok().flatten());
  if let Some(dictionary) = dictionary.and_then(|d| d.resolve().ok().flatten())
  {
    for index in 0..dictionary.dict_len().unwrap_or(0) as i32 {
      let key = dictionary.get_dict_key(index).ok().flatten();
      let value = dictionary.get_dict_val(index).ok().flatten();
      let (Some(key), Some(value)) = (key, value) else { continue };
      let key = key.as_name().unwrap_or_default();
      let key: String = printable(&String::from_utf8_lossy(&key)).collect();
      let value = value.resolve().ok().flatten().unwrap_or(value);
      let Ok(text) = value.as_string() else { continue };
      let text: String = printable(&text).collect();
      let text = text.trim();
      if text.is_empty() {
        continue;
      }
      let text =
        if key.ends_with("Date") { date(text) } else { text.to_owned() };
      info.push((key, text));
    }
  }
  info.sort_by_key(|(key, _)| {
    STANDARD.iter().position(|k| k == key).unwrap_or(STANDARD.len())
  });
  fields.extend(info);
  fields
}

/// `D:20230802120000+02'00'` as `2023-08-02 12:00:00 +02:00`.
fn date(raw: &str) -> String {
  let digits = raw.trim_start_matches("D:");
  let part = |range: std::ops::Range<usize>| {
    digits.get(range).filter(|s| s.chars().all(|c| c.is_ascii_digit()))
  };
  let (Some(year), Some(month), Some(day)) =
    (part(0..4), part(4..6), part(6..8))
  else {
    return raw.to_owned();
  };
  let time = match (part(8..10), part(10..12), part(12..14)) {
    (Some(h), Some(m), Some(s)) => format!(" {h}:{m}:{s}"),
    _ => String::new(),
  };
  let zone = digits.get(14..).unwrap_or("").replace('\'', ":");
  let zone = zone.trim_end_matches(':');
  let zone = if zone.is_empty() { String::new() } else { format!(" {zone}") };
  format!("{year}-{month}-{day}{time}{zone}")
}

/// Drops controls; URL parsers delete newlines.
fn uri(raw: &str) -> String {
  raw.chars().filter(|ch| !ch.is_control()).collect()
}

fn quad_bounds(quad: &mupdf::Quad) -> Bounds {
  Bounds::enclosing(&[quad.ul, quad.ur, quad.ll, quad.lr].map(|p| (p.x, p.y)))
}

fn outline(native: &PdfDocument) -> Result<Vec<Heading>> {
  let mut outline = Vec::new();
  flatten(&native.outlines()?, 0, &mut outline);
  Ok(outline)
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
      .or_else(|| item.uri.as_ref().map(|u| Target::Uri(uri(u))));
    if let Some(target) = target {
      out.push(Heading {
        title: printable(&item.title).collect(),
        depth,
        target,
      });
    }
    flatten(&item.down, depth + 1, out);
  }
}

pub const TEXT_FLAGS: TextPageFlags = TextPageFlags::PRESERVE_LIGATURES
  .union(TextPageFlags::PRESERVE_WHITESPACE)
  .union(TextPageFlags::CLIP)
  .union(TextPageFlags::USE_CID_FOR_UNKNOWN_UNICODE);

pub fn extract(page: &mupdf::Page) -> Result<Text> {
  Ok(read(&page.to_text_page(TEXT_FLAGS)?))
}

/// Text from a display list, which skips interpreting the page again.
pub fn extract_list(list: &DisplayList) -> Result<Text> {
  Ok(read(&list.to_text_page(TEXT_FLAGS)?))
}

fn read(raw: &mupdf::TextPage) -> Text {
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
        row.glyphs.push(Glyph { ch, bounds: quad_bounds(&glyph.quad()) });
        count += 1;
      }
      if row.glyphs.iter().any(|g| !g.ch.is_whitespace()) {
        row.paragraph = paragraph;
        paragraph = false;
        text.lines.push(row);
      }
      if text.truncated {
        return text;
      }
    }
  }
  text
}

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
