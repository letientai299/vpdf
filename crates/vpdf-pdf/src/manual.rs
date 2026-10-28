//! Runtime help pages and composition.

mod destinations;
mod style;
mod writer;

use crate::Result;
use crate::document::Target;
use crate::jobs::{OneShot, Wake};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Chapter {
  pub id: String,
  pub title: String,
  pub lines: Vec<Line>,
  pub related: String,
  pub related_title: String,
  pub section: Option<Section>,
  pub listed: bool,
  pub aliases: Vec<String>,
  pub links: Vec<(String, String)>,
  pub anchors: BTreeMap<usize, String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Section {
  Keys,
  Configuration,
  Commands,
}

impl Section {
  fn title(self) -> &'static str {
    match self {
      Self::Keys => "Built-in keybindings",
      Self::Configuration => "Built-in settings",
      Self::Commands => "Built-in Ex commands",
    }
  }
}

#[derive(Clone, Debug)]
pub enum Line {
  Text(String),
  Binding { keys: String, description: String },
}

impl From<String> for Line {
  fn from(text: String) -> Self {
    Self::Text(text)
  }
}

impl From<&str> for Line {
  fn from(text: &str) -> Self {
    Self::Text(text.into())
  }
}

#[derive(Clone, Debug)]
pub struct Anchor {
  pub topic: String,
  pub name: Option<String>,
  pub target: Target,
}

/// Keep an owner alive until workers close.
pub struct Document {
  file: tempfile::NamedTempFile,
  _private: crate::content_cache::Private,
  pub topics: BTreeMap<String, Target>,
  pub anchors: Vec<Anchor>,
}

impl Document {
  pub fn path(&self) -> &Path {
    self.file.path()
  }

  pub fn anchor_at(
    &self,
    page: crate::text::PageId,
    y: f32,
  ) -> Option<&Anchor> {
    let position = |target: &Target| match target {
      Target::Page(page) => (page.0, 0.0),
      Target::Point { page, y, .. } => (page.0, *y),
      Target::Uri(_) => (usize::MAX, 0.0),
    };
    let first = self
      .anchors
      .iter()
      .filter(|anchor| {
        anchor.name.is_none()
          && position(&anchor.target).0 == page.0
          && position(&anchor.target).1 <= 48.0
      })
      .min_by(|a, b| position(&a.target).1.total_cmp(&position(&b.target).1));
    let boundary = |anchor: &Anchor| {
      let mut position = position(&anchor.target);
      if first.is_some_and(|first| std::ptr::eq(first, anchor)) {
        position.1 = 0.0;
      }
      position
    };
    self.anchors.iter().filter(|anchor| boundary(anchor) <= (page.0, y)).max_by(
      |a, b| {
        let a = boundary(a);
        let b = boundary(b);
        a.0.cmp(&b.0).then_with(|| a.1.total_cmp(&b.1))
      },
    )
  }
}

pub type Job = OneShot<Result<Arc<Document>>>;

pub fn spawn(
  base: &'static [u8],
  palette: vpdf_model::theme::Palette,
  chapters: impl FnOnce() -> Result<Vec<Chapter>> + Send + 'static,
  wake: &Wake,
) -> Job {
  OneShot::spawn(wake, move |stop| {
    let result = chapters().and_then(|chapters| {
      compose(base, &chapters, palette, || {
        !stop.load(std::sync::atomic::Ordering::Relaxed)
      })
    });
    (!stop.load(std::sync::atomic::Ordering::Relaxed)).then_some(result)
  })
}

fn compose(
  base: &[u8],
  chapters: &[Chapter],
  palette: vpdf_model::theme::Palette,
  mut ready: impl FnMut() -> bool,
) -> Result<Arc<Document>> {
  use anyhow::ensure;
  use mupdf::pdf::PdfDocument;

  ensure!(ready(), "Help generation canceled");
  let mut native = PdfDocument::from_copied_bytes(base)?;
  style::authored(&native, palette)?;
  practice_note(&native)?;
  let mut outlines = native.outlines()?;
  let notices = take_notices(&mut native, &mut outlines)?;
  let mut names = destinations::read(&native)?;
  let mut locations = destinations::locations(&names);
  let mut anchored = Vec::new();
  let mut writer = writer::Writer::new(native, palette)?;
  let mut references = BTreeMap::<_, mupdf::Outline>::new();
  for chapter in chapters {
    ensure!(ready(), "Help generation canceled");
    if let Some(section) = chapter.section
      && !references.contains_key(&section)
    {
      let (page, y) = writer.section(5)?;
      references
        .insert(section, destinations::outline_at(section.title(), page, y));
      writer.heading(section.title())?;
    }
    writer.reference_column(chapter, chapters);
    let entry = matches!(chapter.lines.first(), Some(Line::Binding { .. }));
    let (page, y) = writer.section(if entry { 3 } else { 5 })?;
    ensure!(!names.contains_key(&chapter.id), "Duplicate help destination");
    if let Some(section) = chapter.section
      && chapter.listed
    {
      let parent = references.get_mut(&section).expect("Section heading");
      parent.down.push(destinations::outline_at(&chapter.title, page, y));
    }
    anchored.push((chapter.id.clone(), page, y));
    anchored.extend(chapter.aliases.iter().map(|id| (id.clone(), page, y)));
    let related = names
      .get(&chapter.related)
      .ok_or_else(|| anyhow::anyhow!("Missing help teaching destination"))?
      .clone();
    if !entry {
      writer.heading(&chapter.title)?;
      writer.line(&format!("Read: {}", chapter.related_title))?;
      writer.link(related);
    }
    ensure!(
      chapter.anchors.keys().all(|&index| index < chapter.lines.len()),
      "Invalid help anchor line"
    );
    for (index, line) in reference_lines(chapter) {
      ensure!(ready(), "Help generation canceled");
      if let Some(anchor) = chapter.anchors.get(&index) {
        ensure!(
          !anchor.is_empty()
            && anchor
              .bytes()
              .all(|ch| ch.is_ascii_alphanumeric() || b"._-".contains(&ch)),
          "Invalid help anchor"
        );
        let (page, y) = writer.position()?;
        let name = format!("{}.{}", chapter.id, anchor);
        locations.push((
          chapter.id.clone(),
          Some(anchor.clone()),
          name.clone(),
        ));
        anchored.push((name, page, y));
      }
      if let Line::Binding { keys, description } = &line {
        let tags =
          if index == 0 { reference_tags(chapter) } else { Vec::new() };
        writer.entry(keys, description, &tags)?;
      } else {
        writer.reference_line(&line, entry)?;
      }
    }
    if !entry {
      for (label, target) in &chapter.links {
        writer.line(label)?;
        writer.named_link(target.clone());
      }
    }
    if !entry {
      writer.line("")?;
    }
    locations.push((chapter.id.clone(), None, chapter.id.clone()));
  }
  writer.flush()?;
  let mut native = writer.native;
  for (name, page, y) in anchored {
    ensure!(!names.contains_key(&name), "Duplicate help anchor");
    names.insert(name, destinations::point(&native, page, y)?);
  }
  outlines.extend(references.into_values());
  append_notices(&mut native, &mut outlines, notices)?;
  destinations::publish(&native, &names)?;
  destinations::resolve(&native, &names)?;
  native.set_outlines(&outlines)?;
  finish(&native, &names, locations, ready)
}

fn reference_lines(chapter: &Chapter) -> Vec<(usize, Line)> {
  let mut lines: Vec<(usize, Line)> = Vec::new();
  for (index, line) in chapter.lines.iter().enumerate() {
    if !chapter.anchors.contains_key(&index)
      && let Line::Text(text) = line
      && let Some((_, Line::Binding { description, .. })) = lines.last_mut()
    {
      description.push(' ');
      description.push_str(text);
      continue;
    }
    lines.push((index, line.clone()));
  }
  lines
}

fn reference_tags(chapter: &Chapter) -> Vec<String> {
  let mut tags = vec![chapter.related.clone()];
  for (_, target) in &chapter.links {
    if !tags.contains(target) {
      tags.push(target.clone());
    }
  }
  tags
}

fn finish(
  native: &mupdf::pdf::PdfDocument,
  names: &BTreeMap<String, mupdf::pdf::PdfObject>,
  locations: Vec<(String, Option<String>, String)>,
  mut ready: impl FnMut() -> bool,
) -> Result<Arc<Document>> {
  use anyhow::ensure;
  let mut topics = BTreeMap::new();
  for name in names.keys() {
    let dest = native
      .resolve_link(&format!("#nameddest={name}"))?
      .ok_or_else(|| anyhow::anyhow!("Unresolved help destination"))?;
    topics.insert(name.clone(), crate::document::link_target(dest));
  }
  ensure!(ready(), "Help generation canceled");
  let mut file =
    tempfile::Builder::new().prefix("vpdf-help-").suffix(".pdf").tempfile()?;
  let mut options = mupdf::pdf::PdfWriteOptions::default();
  options.set_compress(true).set_compress_fonts(true).set_garbage_level(3);
  native.write_to_with_options(file.as_file_mut(), options)?;
  ensure!(ready(), "Help generation canceled");
  let private = crate::content_cache::Private::new(file.path())?;
  let anchors = locations
    .into_iter()
    .map(|(topic, name, dest)| Anchor {
      topic,
      name,
      target: topics[&dest].clone(),
    })
    .collect();
  Ok(Arc::new(Document { file, _private: private, topics, anchors }))
}

fn append_notices(
  native: &mut mupdf::pdf::PdfDocument,
  outlines: &mut Vec<mupdf::Outline>,
  notices: Vec<mupdf::pdf::PdfObject>,
) -> Result<()> {
  if !notices.is_empty() {
    let page = native.page_count()?;
    for notice in notices {
      native.insert_page(native.page_count()?, &notice)?;
    }
    outlines.push(destinations::outline("Legal notices", page, Vec::new()));
  }
  Ok(())
}

fn take_notices(
  native: &mut mupdf::pdf::PdfDocument,
  outlines: &mut Vec<mupdf::Outline>,
) -> Result<Vec<mupdf::pdf::PdfObject>> {
  let appendix = outlines
    .iter()
    .position(|row| row.title == "Legal notices")
    .and_then(|index| outlines.remove(index).dest)
    .map(|dest| dest.loc.page_number as i32);
  let mut notices = Vec::new();
  if let Some(start) = appendix {
    for page in start..native.page_count()? {
      notices.push(native.find_page(page)?);
    }
    for page in (start..native.page_count()?).rev() {
      native.delete_page(page)?;
    }
  }
  Ok(notices)
}

fn practice_note(native: &mupdf::pdf::PdfDocument) -> Result<()> {
  let Some(target) = native.resolve_link("#nameddest=practice")? else {
    return Ok(());
  };
  let mut page = native.load_pdf_page(target.loc.page_number as i32)?;
  let y = match target.kind {
    mupdf::DestinationKind::XYZ { top, .. } => top.unwrap_or(48.0),
    _ => 48.0,
  };
  let mut note = page.create_annotation(mupdf::pdf::PdfAnnotationType::Text)?;
  note.set_rect(mupdf::Rect::new(564.0, y, 584.0, y + 20.0))?;
  note
    .set_contents("Notes explain passages without changing your source PDF.")?;
  note.set_author("Vpdf")?;
  Ok(())
}

#[cfg(test)]
mod tests;
