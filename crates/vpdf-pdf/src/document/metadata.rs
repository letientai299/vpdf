use super::{
  Document, Heading, PageId, PdfDocument, Result, Target, printable, uri,
};

/// Reads metadata and formats dates.
pub(super) fn metadata(native: &PdfDocument) -> Vec<(String, String)> {
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
pub(super) fn date(raw: &str) -> String {
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

pub(super) fn outline(native: &PdfDocument) -> Result<Vec<Heading>> {
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

impl Document {
  /// Reads the authored outline into `outline`; readers load it on a
  /// worker instead.
  pub fn load_outline(&mut self) -> Result<()> {
    self.outline = outline(&self.native)?;
    Ok(())
  }

  pub fn metadata(&self) -> Vec<(String, String)> {
    metadata(&self.native)
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
}
