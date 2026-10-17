use crate::Result;
use crate::cache::Cache;
use crate::text::{Bounds, Glyph, Line, PageId, Text, printable};
use anyhow::ensure;
use mupdf::pdf::PdfDocument;
use mupdf::{DisplayList, TextPageFlags};
use serde::Serialize;
use std::path::Path;
use std::sync::{Arc, Mutex};

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
  texts: Cache<PageId, Arc<Text>>,
  lists: Arc<Lists>,
  ocr: Cache<PageId, Arc<Text>>,
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

impl Document {
  pub fn open(path: &Path, password: &str) -> Result<Self> {
    let mut native = PdfDocument::open(path)?;
    if native.needs_password()? && !native.authenticate(password)? {
      return Err(PasswordRequired.into());
    }
    let count = native.page_count()?;
    ensure!(count > 0, "PDF contains no pages");
    let mut pages = Vec::with_capacity(count as usize);
    for index in 0..count {
      let bounds = match page_bounds(&native, index) {
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
      pages.push(PageInfo {
        bounds,
        label: native
          .page_label(index as usize)
          .ok()
          .map(|label| printable(&label).collect::<String>())
          .filter(|label| !label.is_empty())
          .unwrap_or_else(|| (index + 1).to_string()),
      });
    }
    let mut outline = Vec::new();
    flatten(&native.outlines()?, 0, &mut outline);
    let title = native
      .metadata(mupdf::MetadataName::Title)
      .map(|title| printable(&title).collect::<String>().trim().to_owned())
      .unwrap_or_default();
    Ok(Self {
      native,
      pages,
      outline,
      title,
      inferred: false,
      texts: Cache::new(32 * MIB),
      lists: Arc::new(Mutex::new(Cache::new(LISTS))),
      ocr: Cache::new(32 * MIB),
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

  /// The display list cache, for another handle of this file.
  pub fn lists(&self) -> Arc<Lists> {
    Arc::clone(&self.lists)
  }

  /// Shares `lists` with the handle that made them.
  pub fn share(&mut self, lists: Arc<Lists>) {
    self.lists = lists;
  }

  /// Drops cached text and display lists; OCR text stays.
  pub fn trim(&mut self) {
    self.texts.clear();
    if let Ok(mut lists) = self.lists.lock() {
      lists.clear();
    }
  }

  pub fn seed(&mut self, page: PageId, text: Arc<Text>) {
    let weight = text.weight();
    self.ocr.insert(page, text, weight);
  }

  pub fn display(&mut self, page: PageId) -> Result<Arc<DisplayList>> {
    self.validate(page)?;
    let cached = self
      .lists
      .lock()
      .ok()
      .and_then(|mut lists| lists.get(&page).map(Arc::clone));
    if let Some(list) = cached {
      return Ok(list);
    }
    let page_data = self.native.load_page(page.0 as i32)?;
    let list = Arc::new(page_data.to_display_list(true)?);
    if let Ok(mut lists) = self.lists.lock() {
      lists.insert(page, Arc::clone(&list), 1);
    }
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
          target: link
            .dest
            .map_or_else(|| Target::Uri(uri(&link.uri)), Target::from),
        })
        .collect(),
    )
  }

  /// Format, encryption, and every Info dictionary entry, standard keys
  /// first; dates read as `YYYY-MM-DD HH:MM:SS`.
  pub fn metadata(&self) -> Vec<(String, String)> {
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
      if let Ok(value) = self.native.metadata(name) {
        fields.push((key.to_owned(), value));
      }
    }
    let mut info = Vec::new();
    let dictionary = self
      .native
      .trailer()
      .ok()
      .and_then(|t| t.get_dict("Info").ok().flatten());
    if let Some(dictionary) =
      dictionary.and_then(|d| d.resolve().ok().flatten())
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

  /// Annotations other than links, popups, and form widgets.
  pub fn annotations(&self, page: PageId) -> Result<Vec<Annotation>> {
    use mupdf::pdf::PdfAnnotationType as Kind;
    self.validate(page)?;
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
      let contents = printable(annotation.contents()?.unwrap_or_default())
        .collect::<String>()
        .trim()
        .to_owned();
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
