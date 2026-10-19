use super::common::keys;
use super::common::{pages_pdf, search_loaded};
use super::focus::screen;
use vpdf::Result;
use vpdf::reference::Reference;
use vpdf::solo::Solo;
use vpdf::text::{PageId, Position};
use vpdf_ui::find::Phase;
use vpdf_ui::keys::PanelKind;

fn open(pages: &[&[&str]], text: bool) -> Result<(tempfile::TempDir, Solo)> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("references.pdf");
  pages_pdf(&path, pages)?;
  let mut app = Solo::open(&path, "", text, screen())?;
  app.target = Some(Position::default());
  Ok((directory, app))
}

#[test]
fn reference_labels_are_exact() {
  let cases = [
    ("See Fig. 4.", "Figure 4: Results", true),
    ("See Figure 4.", "Figure 40: Results", false),
    ("See Figure 4.", "Figure 4 shows the result", false),
    ("See Table 2.", "  Table 2. Results", true),
    ("See Table 2.", "See Table 2 for results", false),
    ("See Section 3.2.", "3.2 Results", true),
    ("See Section 3.2.", "3.20 Results", false),
    ("See Appendix A.", "A Details", false),
    ("See Appendix A.", "Appendix A: Details", true),
    ("See Section A.1.", "A.1 Details", true),
    ("See Section 3.2.", "Chapter 3.2: Other", false),
    ("See Eq. (7).", "x = y (7)", true),
    ("See Eq. (7).", "(7)", true),
    ("See Eq. (7).", "See Eq. (7)", false),
    ("See Eq. (7).", "x = y (70)", false),
  ];
  for (source, target, matches) in cases {
    let reference = Reference::at(source, 4).expect(source);
    assert_eq!(reference.destination(target).is_some(), matches, "{target}");
    let end = source.len() - 2;
    assert_eq!(Reference::at(source, end), Some(reference), "{source}");
    assert!(Reference::at(source, 0).is_none());
  }
  assert!(Reference::at("Section 3.2abc", 0).is_none());
}

#[test]
fn definition_jumps_and_returns() -> Result<()> {
  for text in [false, true] {
    for cursor in [false, true] {
      let (_directory, mut app) = open(
        &[&["See Figure 4."], &["Figure 40: Wrong"], &["Figure 4: Right"]],
        text,
      )?;
      let origin = Position { glyph: 4, ..Position::default() };
      app.target = Some(origin);
      app.reader.cursor = origin;
      if cursor {
        keys(&mut app, b"i")?;
      }
      keys(&mut app, b"gd")?;
      assert_eq!(app.reader.cursor.page, PageId(0));
      search_loaded(&mut app)?;
      assert_eq!(app.reader.cursor.page, PageId(2));
      assert_eq!(app.reader.message, "Reference found");
      keys(&mut app, b"\x1b[111;5u")?;
      assert_eq!(app.reader.cursor.page, PageId(0));
    }
  }
  Ok(())
}

#[test]
fn ambiguous_definitions_offer_candidates() -> Result<()> {
  let (_directory, mut app) =
    open(&[&["Figure 4"], &["Figure 4: First"], &["Figure 4: Second"]], false)?;
  keys(&mut app, b"gd")?;
  search_loaded(&mut app)?;
  assert_eq!(app.reader.cursor.page, PageId(0));
  let panel = app.panel.as_ref().expect("Candidates");
  assert_eq!(panel.view.kind(), PanelKind::Results);
  assert_eq!(panel.view.rows().len(), 2);
  assert!(panel.view.focused());
  keys(&mut app, b"\r")?;
  assert_eq!(app.reader.cursor.page, PageId(1));
  Ok(())
}

#[test]
fn missing_and_canceled_definitions_stay_put() -> Result<()> {
  let (_directory, mut app) =
    open(&[&["Table 2"], &["Table 20: Other"]], false)?;
  keys(&mut app, b"gd")?;
  search_loaded(&mut app)?;
  assert_eq!(app.reader.message, "Reference not found");
  assert_eq!(app.reader.cursor.page, PageId(0));
  for cancel in [b"\x1b[27u".as_slice(), b"j"] {
    app.target = Some(Position::default());
    keys(&mut app, b"gd")?;
    keys(&mut app, cancel)?;
    assert!(matches!(app.search.phase(), Phase::Idle));
    app.search_updates()?;
    assert_eq!(app.reader.cursor.page, PageId(0));
  }
  Ok(())
}

#[test]
fn embedded_definition_keeps_history() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("linked.pdf");
  let content = "BT /F1 18 Tf 20 170 Td (Definition) Tj ET";
  let mut objects = vec![
    "<< /Type /Catalog /Pages 2 0 R >>".into(),
    "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".into(),
  ];
  for annotation in [
    "/Annots [<< /Type /Annot /Subtype /Link /Rect [15 160 280 195] \
     /Dest [4 0 R /Fit] >>]",
    "",
  ] {
    objects.push(format!(
      "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] \
       /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R {annotation} >>"
    ));
  }
  objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
  objects.push(format!(
    "<< /Length {} >>\nstream\n{content}\nendstream",
    content.len()
  ));
  super::common::write_pdf(&path, &objects, "")?;
  let mut app = Solo::open(&path, "", false, screen())?;
  app.target = Some(Position::default());
  keys(&mut app, b"gd")?;
  assert_eq!(app.reader.cursor.page, PageId(1));
  assert!(app.search_worker.is_none());
  keys(&mut app, b"\x1b[111;5u")?;
  assert_eq!(app.reader.cursor.page, PageId(0));
  Ok(())
}

#[test]
fn external_definition_does_not_open_browser() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("external.pdf");
  super::common::linked_pages_pdf(&path, &[&["Figure 4"]], &[(0, 0)])?;
  let mut app = Solo::open(&path, "", false, screen())?;
  app.target = Some(Position::default());
  keys(&mut app, b"gd")?;
  assert_eq!(app.reader.message, "External link: use gx");
  assert!(app.search_worker.is_none());
  Ok(())
}

#[test]
fn section_and_equation_definitions() -> Result<()> {
  for (source, target) in
    [("Section 3.2", "3.2 Methods"), ("Eq. \\(7\\)", "x = y \\(7\\)")]
  {
    let (_directory, mut app) = open(&[&[source], &[target]], false)?;
    keys(&mut app, b"gd")?;
    search_loaded(&mut app)?;
    assert_eq!(app.reader.cursor.page, PageId(1));
    assert_eq!(app.search.hits().len(), 1);
  }
  Ok(())
}

#[test]
fn outline_definition_avoids_scan() -> Result<()> {
  let (_directory, mut app) = open(&[&["Section 3.2"], &["Methods"]], false)?;
  app.doc.outline = vec![vpdf::document::Heading {
    title: "3.2 Methods".into(),
    depth: 1,
    target: vpdf::document::Target::Page(PageId(1)),
  }];
  keys(&mut app, b"gd")?;
  assert_eq!(app.reader.cursor.page, PageId(1));
  assert!(app.search_worker.is_none());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn paper_caption_definition() -> Result<()> {
  let mut app = super::modes::app("paper.pdf", false)?;
  let reference = Reference::at("Table 1", 0).expect("Reference");
  let mut candidates = Vec::new();
  for page in 0..app.doc.pages.len() {
    let page = PageId(page);
    let text = app.doc.text(page)?;
    let index = vpdf::search::Index::new(page, &text);
    candidates.extend(index.definitions(&reference, 100));
  }
  assert_eq!(candidates.len(), 1, "{candidates:?}");
  assert!(candidates[0].excerpt.starts_with("Table 1:"));
  assert!(candidates[0].excerpt.contains("Maximum path"));
  Ok(())
}

#[test]
fn definition_replay_excludes_source() -> Result<()> {
  let (_directory, mut app) =
    open(&[&["Figure 4"], &["Figure 4: Results"]], false)?;
  keys(&mut app, b"gd")?;
  search_loaded(&mut app)?;
  assert_eq!(app.search.hits().len(), 1);
  let query = app.search.query().clone();
  app.rerun(&query)?;
  search_loaded(&mut app)?;
  assert_eq!(app.search.hits().len(), 1);
  assert_eq!(app.search.hits()[0].first.page, PageId(1));
  Ok(())
}

#[test]
fn definition_uses_explicit_heading_titles() -> Result<()> {
  for title in ["Section 3.2 Methods", "3.2"] {
    let (_directory, mut app) = open(&[&["Section 3.2"], &["Methods"]], false)?;
    app.doc.outline = vec![vpdf::document::Heading {
      title: title.into(),
      depth: 1,
      target: vpdf::document::Target::Page(PageId(1)),
    }];
    keys(&mut app, b"gd")?;
    assert_eq!(app.reader.cursor.page, PageId(1), "{title}");
    assert!(app.search_worker.is_none());
  }
  Ok(())
}

#[test]
fn ambiguous_heading_definitions_keep_bookmarks() -> Result<()> {
  let (_directory, mut app) =
    open(&[&["Section 3.2"], &["First"], &["Second"]], false)?;
  app.doc.outline = [1, 2]
    .into_iter()
    .map(|page| vpdf::document::Heading {
      title: "3.2 Methods".into(),
      depth: 1,
      target: vpdf::document::Target::Page(PageId(page)),
    })
    .collect();
  keys(&mut app, b"gd")?;
  assert_eq!(app.reader.cursor.page, PageId(0));
  let panel = app.panel.as_ref().expect("Bookmark candidates");
  assert_eq!(panel.view.kind(), PanelKind::Outline);
  assert_eq!(panel.view.rows().len(), 2);
  assert!(panel.view.focused());
  keys(&mut app, b"j\r")?;
  assert_eq!(app.reader.cursor.page, PageId(2));
  Ok(())
}

#[test]
fn definition_labels_reject_prose() {
  let appendix = Reference::at("Appendix A", 0).expect("Appendix");
  assert!(appendix.destination("A useful result follows").is_none());
  assert!(appendix.heading("A Details"));
  let equation = Reference::at("Eq. (7)", 0).expect("Equation");
  assert!(equation.destination("We use x = y in Eq. (7)").is_none());
  let appendix = Reference::at("Section A.1", 0).expect("Compound label");
  assert!(appendix.destination("A.1 Methods").is_some());
  let table = Reference::at("Table 2", 0).expect("Table");
  assert!(table.destination("Table 2 : Results").is_some());
}

#[test]
fn definition_focus_loss_cancels_jump() -> Result<()> {
  let (_directory, mut app) =
    open(&[&["Figure 4"], &["Figure 4: Results"]], false)?;
  keys(&mut app, b"gd")?;
  app.blur()?;
  assert!(matches!(app.search.phase(), Phase::Idle));
  app.search_updates()?;
  assert_eq!(app.reader.cursor.page, PageId(0));
  Ok(())
}

#[test]
fn definition_overrides_pending_search_jump() -> Result<()> {
  let (_directory, mut app) =
    open(&[&["Section 3.2"], &["Methods"], &["needle"]], false)?;
  app.doc.outline = vec![vpdf::document::Heading {
    title: "3.2 Methods".into(),
    depth: 1,
    target: vpdf::document::Target::Page(PageId(1)),
  }];
  app.start_search("needle", false, false)?;
  keys(&mut app, b"gd")?;
  assert_eq!(app.reader.cursor.page, PageId(1));
  search_loaded(&mut app)?;
  assert_eq!(app.reader.cursor.page, PageId(1));
  Ok(())
}
