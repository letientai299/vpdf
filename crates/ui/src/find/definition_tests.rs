use super::*;
use crate::Result;
use crate::source::Outline;
use crate::text::{Bounds, Glyph, Line, PageId, Text};
use std::sync::Arc;
use vpdf_model::document::{Heading, Target};
use vpdf_reader::reader::TextSource;

struct Source {
  text: Option<Arc<Text>>,
  headings: Vec<Heading>,
  reads: usize,
}

impl TextSource for Source {
  fn page_count(&self) -> usize {
    3
  }
  fn page_text(&mut self, _: PageId) -> Result<Arc<Text>> {
    self.reads += 1;
    self.text.clone().ok_or_else(|| anyhow::anyhow!("Text unavailable"))
  }
}

impl Outline for Source {
  fn outline(&self) -> &[Heading] {
    &self.headings
  }
}

fn source(value: &str) -> Source {
  let glyphs =
    value.chars().map(|ch| Glyph { ch, bounds: Bounds::default() }).collect();
  Source {
    text: Some(Arc::new(Text {
      lines: vec![Line { paragraph: false, glyphs }],
      truncated: false,
    })),
    headings: Vec::new(),
    reads: 0,
  }
}

#[test]
fn links_skip_text_reads() -> Result<()> {
  let mut source = Source { text: None, headings: Vec::new(), reads: 0 };
  let position = Position::default();
  let target = Target::Page(PageId(1));
  assert_eq!(
    Definition::resolve(position, Some(target.clone()), &mut source)?,
    Definition::Follow(target)
  );
  let external = Definition::resolve(
    position,
    Some(Target::Uri("https://example.org".into())),
    &mut source,
  )?;
  assert_eq!(external, Definition::External);
  assert_eq!(external.message(), Some("External link: use gx"));
  assert_eq!(source.reads, 0);
  assert!(Definition::resolve(position, None, &mut source).is_err());
  assert_eq!(source.reads, 1);
  Ok(())
}

#[test]
fn references_choose_destinations() -> Result<()> {
  let mut source = source("See Section 2.");
  let origin = Position { glyph: 4, ..Position::default() };
  let Definition::Search(lookup) =
    Definition::resolve(origin, None, &mut source)?
  else {
    panic!("Missing reference lookup")
  };
  assert_eq!(lookup.origin, origin);
  assert_eq!(lookup.reference.title(), "section 2");
  for page in [1, 1] {
    source.headings.push(Heading {
      title: "2 Methods".into(),
      depth: 1,
      target: Target::Page(PageId(page)),
    });
  }
  assert_eq!(
    Definition::resolve(origin, None, &mut source)?,
    Definition::Follow(Target::Page(PageId(1)))
  );
  source.headings.push(Heading {
    title: "2 More".into(),
    depth: 1,
    target: Target::Page(PageId(2)),
  });
  assert_eq!(
    Definition::resolve(origin, None, &mut source)?,
    Definition::Outline(0)
  );
  Ok(())
}

#[test]
fn absent_targets_keep_notices() -> Result<()> {
  let mut source = source("No reference here");
  let missing = Definition::resolve(Position::default(), None, &mut source)?;
  assert_eq!(missing, Definition::NoReference);
  assert_eq!(missing.message(), Some("No document reference at target"));
  let absent = Position { line: 1, ..Position::default() };
  let unchanged = Definition::resolve(absent, None, &mut source)?;
  assert_eq!(unchanged, Definition::Unchanged);
  assert_eq!(unchanged.message(), None);
  Ok(())
}

#[test]
fn results_choose_navigation() {
  let mut search = Search::default();
  assert_eq!(search.definition_result(), Definition::NotFound);
  let position = Position { page: PageId(1), ..Position::default() };
  let hit =
    Hit { first: position, last: position, excerpt: "Figure 1: Data".into() };
  let query = Query { text: "figure 1".into(), ..Query::default() };
  search.show(query.clone(), vec![hit.clone()]);
  let definition = search.definition_result();
  assert_eq!(
    definition,
    Definition::Found { position, description: search.description(position) }
  );
  assert_eq!(definition.message(), Some("Reference found"));
  search
    .show(query, vec![hit.clone(), Hit { first: Position::default(), ..hit }]);
  assert_eq!(search.definition_result(), Definition::Candidates);
}
