use crate::common;
use crate::focus::screen;
use crate::harness::{corpus, solo};
use std::sync::Arc;
use vpdf::Result;
use vpdf::app::App;
use vpdf::solo::Solo;
use vpdf_model::document::Target;
use vpdf_model::text::{Bounds, Glyph, Line, PageId, Text};
use vpdf_ui::keys::PanelKind;
use vpdf_ui::panel::Destination;
use vpdf_ui::pointer::{Hit, Region};

/// A target as text.
fn address(target: &Target) -> String {
  match target {
    Target::Uri(uri) => uri.clone(),
    other => format!("{other:?}"),
  }
}

/// Targets and anchor text of page 1's links.
fn links(app: &mut App) -> Result<Vec<(String, String)>> {
  let page = PageId(0);
  let text = app.doc.text(page)?;
  let links = app.doc.links(page)?;
  let found =
    links.iter().map(|link| (address(&link.target), link.anchor(&text, page)));
  Ok(found.collect())
}

/// One page of 12 pt Courier `lines` with web links over `linked` lines;
/// equal lengths share a right margin.
fn open(
  lines: &[&str],
  linked: &[(usize, usize)],
) -> Result<(tempfile::TempDir, Solo)> {
  let (directory, mut app) = common::temp_solo(
    |path| common::pages_of(path, &[lines], ("Courier", 12, 24), linked),
    false,
    screen(),
  )?;
  app.draw(&mut Vec::new())?;
  Ok((directory, app))
}

/// Lines, web links over lines, then each link's target and anchor.
type Case = (
  &'static [&'static str],
  &'static [(usize, usize)],
  &'static [(&'static str, &'static str)],
);

#[test]
fn printed_addresses_are_links() -> Result<()> {
  let cases: [Case; 3] = [
    (
      &["Read https://a.org/x.", "or doi:10.1000/abc now", "e.g. version 10.5"],
      &[],
      &[
        ("https://a.org/x", "https://a.org/x"),
        ("https://doi.org/10.1000/abc", "doi:10.1000/abc"),
      ],
    ),
    (
      &[
        "go https://a.org/very/long/",
        "path/x ok and the rows end.",
        "a third row of equal width.",
      ],
      &[],
      &[("https://a.org/very/long/path/x", "https://a.org/very/long/path/x")],
    ),
    (
      &["Visit https://b.org/ now", "or www.c.org"],
      &[(0, 0)],
      &[
        ("https://example.com/", "Visit https://b.org/ now"),
        ("http://www.c.org", "www.c.org"),
      ],
    ),
  ];
  for (lines, linked, want) in cases {
    let (_directory, mut app) = open(lines, linked)?;
    let want: Vec<_> = want
      .iter()
      .map(|(target, anchor)| ((*target).to_owned(), (*anchor).to_owned()))
      .collect();
    assert_eq!(links(&mut app)?, want, "{lines:?}");
  }
  Ok(())
}

#[test]
fn printed_links_reach_every_consumer() -> Result<()> {
  let (_directory, mut app) =
    open(&["Read https://a.org/x.", "or doi:10.1000/abc now"], &[])?;
  let doi = "https://doi.org/10.1000/abc";
  let is_doi =
    |target: &Target| matches!(target, Target::Uri(uri) if uri == doi);
  // Hover finds both.
  let mut hits = Vec::new();
  for row in 0..app.screen.rows - 1 {
    for column in 0..app.screen.columns {
      let hit = app.hovered(Region::Page, column, row, None)?;
      if matches!(hit, Hit::Link(..)) && !hits.contains(&hit) {
        hits.push(hit);
      }
    }
  }
  assert_eq!(hits, [Hit::Link(PageId(0), 0), Hit::Link(PageId(0), 1)]);
  // `gl` lists them.
  app.open_panel(PanelKind::Links)?;
  common::sidebar_loaded(&mut app, PanelKind::Links)?;
  let rows = &app.panel.as_ref().expect("link panel").view.rows();
  let labels: Vec<_> = rows.iter().map(|row| row.label.as_str()).collect();
  assert_eq!(labels, ["https://a.org/x", "doi:10.1000/abc"]);
  assert!(matches!(&rows[1].destination, Destination::Link(t) if is_doi(t)));
  app.close_panel()?;
  // `yf` labels them.
  common::keys(&mut app, b"yf")?;
  assert!(app.reader.operator.is_none());
  let labels = app.overlays.hints().expect("hints").items();
  assert_eq!(labels.len(), 2);
  let label =
    labels.iter().find(|hint| is_doi(&hint.target)).unwrap().label.clone();
  common::keys(&mut app, label.as_bytes())?;
  assert!(app.overlays.hints().is_none());
  assert_eq!(app.reader.message, "Copied link");
  let copied = app.reader.take_yank().expect("copied URL");
  assert_eq!(copied.text, doi);
  Ok(())
}

#[test]
fn seeded_text_finds_links_again() -> Result<()> {
  let (_directory, mut app) = open(&["plain words"], &[])?;
  assert_eq!(links(&mut app)?, Vec::<(String, String)>::new());
  let glyphs = "see https://x.org".chars().zip(0..).map(|(ch, index)| {
    let x0 = 20.0 + index as f32 * 7.0;
    Glyph {
      quad: None,
      ch,
      bounds: Bounds { x0, y0: 20.0, x1: x0 + 7.0, y1: 32.0 },
    }
  });
  let line = Line { ocr: None, paragraph: true, glyphs: glyphs.collect() };
  let text = Text { lines: vec![line], truncated: false };
  app.doc.seed(PageId(0), Arc::new(text));
  let want = [("https://x.org".to_owned(), "https://x.org".to_owned())];
  assert_eq!(links(&mut app)?, want);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn real_preprint_addresses_are_links() -> Result<()> {
  let path = corpus("review.pdf")?;
  let mut app = Solo::open(&path, "", false, screen())?;
  // page, target, anchors
  let cases: [(usize, &str, &[&str]); 3] = [
    (
      16,
      "https://doi.org/10.5194/egusphere-2025-2374",
      &["https://doi.org/10.5194/egusphere-2025-2374"],
    ),
    // Wrapped past a margin line number.
    (
      20,
      "https://doi.org/10.5194/acp-16-5283-2016",
      &["https://doi.org/10.5194/acp-16-5283-2016"],
    ),
    (
      0,
      "mailto:hjuchem@iup.uni-heidelberg.de",
      &["hjuchem@iup.uni-heidelberg.de"],
    ),
  ];
  for (page, target, anchors) in cases {
    let page = PageId(page);
    let text = app.doc.text(page)?;
    let links = app.doc.links(page)?;
    let found: Vec<_> = links
      .iter()
      .filter(|link| address(&link.target) == target)
      .map(|link| link.anchor(&text, page))
      .collect();
    assert_eq!(found, anchors, "{target}");
  }
  app.command("17")?;
  app.open_panel(PanelKind::Links)?;
  common::sidebar_loaded(&mut app, PanelKind::Links)?;
  let rows = &app.panel.as_ref().expect("link panel").view.rows();
  assert!(rows.iter().any(|row| row.label == cases[0].1), "gl lists the DOI");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn paper_author_addresses_are_links() -> Result<()> {
  let mut doc = vpdf_pdf::document::Document::open(&corpus("paper.pdf")?, "")?;
  let links = doc.links(PageId(0))?;
  let mut mail: Vec<_> = links
    .iter()
    .map(|link| address(&link.target))
    .filter(|target| target.starts_with("mailto:"))
    .collect();
  mail.dedup();
  assert_eq!(
    mail,
    [
      "mailto:avaswani@google.com",
      "mailto:noam@google.com",
      "mailto:nikip@google.com",
      "mailto:usz@google.com",
      "mailto:llion@google.com",
      "mailto:aidan@cs.toronto.edu",
      "mailto:lukaszkaiser@google.com",
      "mailto:illia.polosukhin@gmail.com",
    ]
  );
  Ok(())
}

/// Page wraps skip headers and whole mentions.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn real_page_wraps_tint_the_continuation() -> Result<()> {
  let mut app = solo("book.pdf", false, screen())?;
  // (page, address end) -> tinted next-page tops
  let cases: [(usize, &str, &[f32]); 2] =
    [(122, "interlock.py", &[88.0]), (58, "polygon.py", &[])];
  for (page, end, want) in cases {
    let page = PageId(page);
    let links = app.doc.links(page)?;
    let index =
      links.iter().position(|link| address(&link.target).ends_with(end));
    app.hover.point(
      Hit::Link(page, index.expect("the address")),
      None,
      std::time::Instant::now,
    );
    let next: Vec<_> = (app.hover_segments().into_iter())
      .filter(|(on, _)| on.0 == page.0 + 1)
      .map(|(_, bounds)| bounds.y0.round())
      .collect();
    assert_eq!(next, want, "{end}");
  }
  Ok(())
}
