use super::*;

fn compose(
  base: &[u8],
  chapters: &[Chapter],
  ready: impl FnMut() -> bool,
) -> Result<Arc<Document>> {
  super::compose(
    base,
    chapters,
    vpdf_model::theme::Theme::default().palette(),
    ready,
  )
}
use mupdf::pdf::PdfDocument;

fn base() -> Result<Vec<u8>> {
  let source = tempfile::NamedTempFile::new()?;
  crate::fixtures::pdf(source.path(), &["Authored paragraph"], "")?;
  let native = PdfDocument::open(source.path())?;
  let mut pairs = native.new_array()?;
  pairs.array_push(native.new_string("introduction")?)?;
  pairs.array_push(destinations::point(&native, 0, 0.0)?)?;
  let mut dests = native.new_dict()?;
  dests.dict_put("Names", pairs)?;
  let mut names = native.new_dict()?;
  names.dict_put("Dests", dests)?;
  native.catalog()?.dict_put("Names", names)?;
  let mut bytes = Vec::new();
  native.write_to(&mut bytes)?;
  Ok(bytes)
}

#[test]
fn long_reference_roundtrip() -> Result<()> {
  let base = base()?;
  let mut chapters = vec![Chapter {
    id: "half_down".into(),
    title: "Half page down".into(),
    related: "introduction".into(),
    related_title: "Introduction".into(),
    section: None,
    listed: true,
    aliases: Vec::new(),
    links: Vec::new(),
    anchors: BTreeMap::new(),
    lines: vec![
      "café Ελληνικά <C-d> h H g g".into(),
      "Missing glyph: 文; literal: \\u{6587}; control: \n".into(),
    ],
  }];
  for row in 0..180 {
    chapters[0].lines.push(
      format!(
        "Binding {row:03}: {}END{row:03}",
        "/configuration/directory/".repeat(12)
      )
      .into(),
    );
  }
  chapters.push(Chapter {
    id: "config".into(),
    title: "Configuration".into(),
    related: "introduction".into(),
    related_title: "Introduction".into(),
    section: None,
    listed: true,
    aliases: Vec::new(),
    links: Vec::new(),
    anchors: BTreeMap::new(),
    lines: vec!["Effective: snapshot value".into()],
  });
  let document = compose(&base, &chapters, || true)?;
  let native = PdfDocument::open(document.path())?;
  assert!(native.page_count()? > 10);
  let mut text = String::new();
  for index in 0..native.page_count()? {
    text.push_str(
      &native
        .load_page(index)?
        .to_text_page(mupdf::TextPageFlags::empty())?
        .to_text()?,
    );
  }
  assert!(text.contains("café Ελληνικά <C-d> h H g g"));
  assert!(text.contains("Missing glyph: \\u{6587}"));
  assert!(text.contains("literal: \\\\u{6587}"));
  assert!(text.contains("control: \\u{a}"));
  let flat: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
  let mut at = 0;
  for row in 0..180 {
    let needle = format!(
      "Binding{row:03}:{}END{row:03}",
      "/configuration/directory/".repeat(12)
    );
    let found = flat.find(&needle).unwrap();
    assert!(found >= at);
    at = found;
  }
  for id in ["introduction", "half_down", "config"] {
    assert!(native.resolve_link(&format!("#nameddest={id}"))?.is_some());
  }
  assert_eq!(native.load_page(1)?.links()?.count(), 1);
  Ok(())
}

#[test]
fn rejects_invalid_and_canceled() -> Result<()> {
  let base = base()?;
  let chapter = Chapter {
    id: "introduction".into(),
    title: "Duplicate".into(),
    related: "introduction".into(),
    related_title: "Introduction".into(),
    section: None,
    listed: true,
    aliases: Vec::new(),
    links: Vec::new(),
    anchors: BTreeMap::new(),
    lines: Vec::new(),
  };
  assert!(compose(&base, &[chapter], || true).is_err());
  let chapter = Chapter {
    id: "config".into(),
    title: "Configuration".into(),
    related: "missing".into(),
    related_title: "Missing".into(),
    section: None,
    listed: true,
    aliases: Vec::new(),
    links: Vec::new(),
    anchors: BTreeMap::new(),
    lines: Vec::new(),
  };
  assert!(compose(&base, &[chapter], || true).is_err());
  assert!(compose(&base, &[], || false).is_err());
  let mut steps = 0;
  assert!(
    compose(&base, &[], || {
      steps += 1;
      steps < 3
    })
    .is_err()
  );
  assert!(compose(b"Invalid PDF", &[], || true).is_err());
  Ok(())
}

#[test]
fn private_files_skip_records() -> Result<()> {
  use crate::content_cache::{Kind, Record, cacheable};
  use std::os::unix::fs::PermissionsExt;
  let doc = compose(&base()?, &[], || true)?;
  assert_eq!(
    std::fs::metadata(doc.path())?.permissions().mode() & 0o777,
    0o600
  );
  assert!(!cacheable(doc.path(), ""));
  let root = tempfile::tempdir()?;
  let alias = root.path().join("alias.pdf");
  std::os::unix::fs::symlink(doc.path(), &alias)?;
  assert!(!cacheable(&alias, ""));
  for source in [doc.path(), &alias] {
    assert!(
      Record::open(
        root.path().into(),
        source,
        "",
        Kind::Outline,
        "test",
        || true
      )
      .is_none()
    );
  }
  let handle = crate::document::Handle::open(doc.path(), "")?;
  assert!(!handle.cacheable);
  let normal = tempfile::NamedTempFile::new()?;
  assert!(cacheable(normal.path(), ""));
  assert!(!cacheable(normal.path(), "password"));
  Ok(())
}

#[test]
fn entries_join_wrapped_descriptions() -> Result<()> {
  let description = "Edit the PDF in this tab. ".repeat(160);
  let chapter = Chapter {
    id: "ex.edit".into(),
    title: ":edit".into(),
    related: "introduction".into(),
    related_title: "Introduction".into(),
    section: None,
    listed: true,
    aliases: Vec::new(),
    links: vec![("Introduction".into(), "introduction".into())],
    anchors: BTreeMap::from([(1, "details".into())]),
    lines: vec![
      Line::Binding {
        keys: ":e[dit] <file>".into(),
        description: description.clone(),
      },
      "Available in document views.".into(),
    ],
  };
  let document = compose(&base()?, &[chapter], || true)?;
  let handle = crate::document::Handle::open(document.path(), "")?;
  let mut rendered = String::new();
  let mut links = 0;
  let mut prose_end = None;
  for index in 1..handle.native.page_count()? {
    for line in &handle.text(crate::text::PageId(index as usize))?.lines {
      let Some(first) = line.glyphs.first() else { continue };
      let text: String = line.glyphs.iter().map(|glyph| glyph.ch).collect();
      let expected = if text.starts_with("Available in") { 60.0 } else { 48.0 };
      assert!((first.bounds.x0 - expected).abs() < 1.0, "{text}");
      if text.len() > 60 && prose_end.is_none() {
        prose_end = line.glyphs.last().map(|glyph| glyph.bounds.x1);
      }
      rendered.push_str(&text);
      rendered.push(' ');
    }
    links += handle.native.load_page(index)?.links()?.count();
  }
  assert!(rendered.starts_with(":e[dit] <file>  Edit the PDF in this tab."));
  assert!(rendered.contains(":help introduction"));
  assert!(!rendered.contains("See |"));
  assert!(rendered.contains("Available in document views."));
  assert!((prose_end.unwrap() - 564.0).abs() < 2.5);
  assert!(!rendered.contains("== :edit =="));
  assert!(!rendered.contains("Read:"));
  let flat = rendered.split_whitespace().collect::<String>();
  assert!(flat.contains(&description.split_whitespace().collect::<String>()));
  assert_eq!(links, 1);
  assert!(document.topics.contains_key("ex.edit.details"));
  Ok(())
}

#[test]
fn section_destination_includes_unlisted_opening() -> Result<()> {
  let first = Chapter {
    id: "unlisted".into(),
    title: "Unlisted entry".into(),
    lines: vec!["Unlisted opening content".into(); 100],
    related: "introduction".into(),
    related_title: "Introduction".into(),
    section: Some(Section::Keys),
    listed: false,
    aliases: vec!["unlisted.alias".into()],
    links: Vec::new(),
    anchors: BTreeMap::new(),
  };
  let mut second = first.clone();
  second.id = "listed".into();
  second.title = "Listed entry".into();
  second.listed = true;
  second.aliases.clear();
  second.lines = vec!["Listed content".into()];
  let document = compose(&base()?, &[first, second], || true)?;
  let native = PdfDocument::open(document.path())?;
  let outlines = native.outlines()?;
  let section =
    outlines.iter().find(|row| row.title == "Built-in keybindings").unwrap();
  let page = |id: &str| match document.topics[id] {
    Target::Point { page, .. } | Target::Page(page) => page.0 as u32,
    Target::Uri(_) => panic!("External destination"),
  };
  assert_eq!(section.dest.unwrap().loc.page_number, page("unlisted"));
  assert!(page("listed") > page("unlisted"));
  assert_eq!(section.down.len(), 1);
  assert_eq!(section.down[0].title, "Listed entry");
  assert_eq!(document.topics["unlisted"], document.topics["unlisted.alias"]);
  Ok(())
}

#[test]
fn entries_preserve_literal_keys_and_links() -> Result<()> {
  let chapter = Chapter {
    id: "intro".into(),
    title: "Literal keys".into(),
    related: "introduction".into(),
    related_title: "Introduction".into(),
    section: None,
    listed: true,
    aliases: Vec::new(),
    links: vec![("Self".into(), "intro".into())],
    anchors: BTreeMap::new(),
    lines: vec![
      Line::Binding {
        keys: "`{mark}".into(),
        description: "Jump to the saved glyph.".into(),
      },
      "The view follows the saved position.".into(),
    ],
  };
  let document = compose(&base()?, &[chapter], || true)?;
  let native = PdfDocument::open(document.path())?;
  let page = native.load_page(1)?;
  let text = page.to_text_page(mupdf::TextPageFlags::empty())?.to_text()?;
  assert!(text.contains("`{mark}  Jump to the saved glyph."), "{text}");
  assert!(text.contains("The view follows the saved position."));
  assert!(!text.contains("See |"));
  let links: Vec<_> = page.links()?.collect();
  assert_eq!(links.len(), 2);
  assert_ne!(links[0].dest, links[1].dest);
  for link in &links {
    assert!(link.bounds.width() < 160.0, "{link:?}");
  }
  assert!(
    links[0].bounds.x1 <= links[1].bounds.x0
      || links[1].bounds.y0 > links[0].bounds.y0
  );
  Ok(())
}
