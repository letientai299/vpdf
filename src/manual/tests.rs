use super::*;
use mupdf::{TextPageFlags, pdf::PdfDocument};

mod runtime;

#[test]
fn picker_help_describes_frontends() -> crate::Result<()> {
  use vpdf_pdf::manual::Line;
  use vpdf_ui::commands::{COMMANDS, Id};
  let chapters = reference::chapters()?;
  for id in [Id::Fzf, Id::Edit, Id::Open, Id::TabEdit] {
    let topic = COMMANDS.iter().find(|spec| spec.id == id).unwrap().topic();
    let chapter = chapters.iter().find(|chapter| chapter.id == topic).unwrap();
    let text = chapter
      .lines
      .iter()
      .filter_map(|line| match line {
        Line::Text(text) => Some(text.as_str()),
        Line::Binding { .. } => None,
      })
      .collect::<String>();
    assert!(text.contains("system file picker"), "{topic}: {text}");
    assert!(text.contains("fzf"), "{topic}: {text}");
    if id == Id::Open {
      assert!(text.contains("Tab completion"), "{text}");
    }
  }
  Ok(())
}

#[test]
fn embedded_topics_resolve() -> crate::Result<()> {
  let native = PdfDocument::from_copied_bytes(PDF)?;
  let topics: serde_json::Value = serde_json::from_str(TOPICS)?;
  let mut text = String::new();
  let mut dynamic = std::collections::BTreeSet::new();
  let mut local_links = 0;
  for index in 0..native.page_count()? {
    let page = native.load_page(index)?;
    text.push_str(&page.to_text_page(TextPageFlags::empty())?.to_text()?);
    for link in page.links()? {
      if let Some(topic) = link.uri.strip_prefix("vpdf-help:") {
        dynamic.insert(topic.to_owned());
      } else if link.uri.starts_with("https://") {
        assert!(link.dest.is_none());
      } else {
        let target = link.dest.expect("Internal destination");
        assert!(target.loc.page_number < native.page_count()? as u32);
        local_links += 1;
      }
    }
  }
  assert!(local_links >= 3);
  assert!(dynamic.contains("half_down"));
  assert!(dynamic.contains("config"));
  assert_eq!(topics["authored"][0]["id"], "introduction");
  assert_eq!(topics["authored"][0]["page"], 0);
  for topic in topics["authored"].as_array().unwrap() {
    let id = topic["id"].as_str().unwrap();
    let target = native.resolve_link(&format!("#nameddest={id}"))?.unwrap();
    assert_eq!(u64::from(target.loc.page_number), topic["page"]);
    let title = topic["title"].as_str().unwrap();
    let page = native.load_page(target.loc.page_number as i32)?;
    assert!(
      page.to_text_page(TextPageFlags::empty())?.to_text()?.contains(title)
    );
  }
  for needle in
    ["café", "cà phê", "Ελληνικά", "SIL OPEN FONT LICENSE", "Bitstream"]
  {
    assert!(text.contains(needle), "Missing embedded text: {needle}");
  }
  let outlines = native.outlines()?;
  assert!(outlines.iter().any(|row| !row.down.is_empty()));
  assert_eq!(identity().len(), 64);
  Ok(())
}

#[test]
fn embedded_links_have_distinct_hints() -> crate::Result<()> {
  let native = PdfDocument::from_copied_bytes(PDF)?;
  for index in 0..native.page_count()? {
    let links: Vec<_> = native.load_page(index)?.links()?.collect();
    for (position, link) in links.iter().enumerate() {
      for other in &links[..position] {
        assert!(
          link.uri != other.uri
            || (link.bounds.y0 - other.bounds.y0).abs() > 0.1
            || ((link.bounds.x0 - other.bounds.x1).abs() > 0.1
              && (other.bounds.x0 - link.bounds.x1).abs() > 0.1),
          "Adjacent hint targets for one visible link: {link:?}"
        );
      }
    }
  }
  Ok(())
}
