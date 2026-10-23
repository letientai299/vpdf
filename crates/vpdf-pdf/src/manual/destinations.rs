use crate::Result;
use anyhow::{Context, ensure};
use mupdf::pdf::{PdfDocument, PdfObject};
use mupdf::{DestinationKind, Outline};
use std::collections::BTreeMap;

pub(super) fn read(
  native: &PdfDocument,
) -> Result<BTreeMap<String, PdfObject>> {
  let flat = native.load_name_tree(native.new_name("Dests")?)?;
  let mut out = BTreeMap::new();
  for entry in flat.dict_iter()? {
    let (key, value) = entry?;
    out.insert(String::from_utf8(key.as_name()?)?, value);
  }
  Ok(out)
}

pub(super) fn locations(
  names: &BTreeMap<String, PdfObject>,
) -> Vec<(String, Option<String>, String)> {
  names
    .keys()
    .map(|id| {
      let (topic, anchor) = id
        .split_once('.')
        .filter(|(topic, _)| names.contains_key(*topic))
        .map_or((id.as_str(), None), |(topic, anchor)| {
          (topic, Some(anchor.to_owned()))
        });
      (topic.to_owned(), anchor, id.clone())
    })
    .collect()
}

pub(super) fn point(
  native: &PdfDocument,
  page: i32,
  y: f32,
) -> Result<PdfObject> {
  let mut dest = native.new_array()?;
  dest.array_push(native.find_page(page)?)?;
  dest.array_push(native.new_name("XYZ")?)?;
  dest.array_push(native.new_real(48.0)?)?;
  dest.array_push(native.new_real(792.0 - y)?)?;
  dest.array_push(native.new_null())?;
  Ok(dest)
}

pub(super) fn outline(title: &str, page: i32, down: Vec<Outline>) -> Outline {
  Outline {
    title: title.into(),
    uri: None,
    dest: Some(mupdf::link::LinkDestination {
      loc: mupdf::document::Location {
        chapter: 0,
        page_in_chapter: page as u32,
        page_number: page as u32,
      },
      kind: DestinationKind::Fit,
    }),
    down,
  }
}

pub(super) fn outline_at(title: &str, page: i32, y: f32) -> Outline {
  let mut outline = outline(title, page, Vec::new());
  outline.dest.as_mut().expect("Reference destination").kind =
    DestinationKind::XYZ { left: Some(48.0), top: Some(y), zoom: None };
  outline
}

pub(super) fn publish(
  native: &PdfDocument,
  entries: &BTreeMap<String, PdfObject>,
) -> Result<()> {
  let mut array = native.new_array()?;
  for (name, dest) in entries {
    array.array_push(native.new_string(name)?)?;
    array.array_push(dest.clone())?;
  }
  let mut dests = native.new_dict()?;
  dests.dict_put("Names", array)?;
  let mut names =
    native.catalog()?.get_dict("Names")?.context("Help names missing")?;
  names.dict_put("Dests", dests)?;
  Ok(())
}

pub(super) fn resolve(
  native: &PdfDocument,
  names: &BTreeMap<String, PdfObject>,
) -> Result<()> {
  for page in 0..native.page_count()? {
    let Some(annots) = native.find_page(page)?.get_dict("Annots")? else {
      continue;
    };
    for annot in annots.array_iter()? {
      let mut annot = annot?;
      let Some(action) = annot.get_dict("A")? else { continue };
      let Some(uri) = action.get_dict("URI")? else { continue };
      let uri = uri.as_string()?;
      let Some(topic) = uri.strip_prefix("vpdf-help:") else { continue };
      let dest = names.get(topic).context("Unknown help link destination")?;
      annot.dict_delete("A")?;
      annot.dict_put("Dest", dest.clone())?;
    }
    for link in native.load_page(page)?.links()? {
      if link.uri.starts_with('#') || link.uri.starts_with("vpdf-help:") {
        ensure!(link.dest.is_some(), "Unresolved help link");
      }
    }
  }
  Ok(())
}
