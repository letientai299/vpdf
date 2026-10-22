use super::*;
use crate::text::{Bounds, Glyph, PageId};

#[test]
fn glyph_offsets_preserve_unicode() {
  let value = "é 🧬 See Figure 4.";
  let line = Line {
    ocr: None,
    paragraph: false,
    glyphs: value
      .chars()
      .map(|ch| Glyph { quad: None, ch, bounds: Bounds::default() })
      .collect(),
  };
  let start = value.chars().position(|ch| ch == 'F').unwrap();
  let reference = Reference::at("Figure 4", 0).unwrap();
  for glyph in 0..=line.glyphs.len() + 1 {
    assert_eq!(
      Reference::at_glyph(&line, glyph),
      (start..start + 8).contains(&glyph).then(|| reference.clone())
    );
  }
  let origin = Position { page: PageId(2), line: 3, glyph: start };
  let lookup = Lookup { reference, origin };
  let query = crate::query::Query::from(lookup.clone());
  assert_eq!(query.text, "figure 4");
  assert_eq!(query.reference, Some(lookup));
  assert!(!query.backward && !query.whole);
}

#[test]
fn headings_keep_unique_targets() {
  let reference = Reference::at("Section 2", 0).unwrap();
  let point = Target::Point { page: PageId(1), x: 10.0, y: 20.0 };
  let outline: Vec<_> = [
    ("1 Other", Target::Page(PageId(0))),
    ("2 External", Target::Uri("https://example.org".into())),
    ("2 Missing", Target::Page(PageId(9))),
    ("2 Methods", Target::Page(PageId(1))),
    ("2 Alias", Target::Page(PageId(1))),
    ("2 Detail", point.clone()),
    ("2 Same detail", point),
  ]
  .into_iter()
  .map(|(title, target)| Heading { title: title.into(), depth: 1, target })
  .collect();
  assert_eq!(
    reference.headings(&outline, 3),
    vec![(3, &outline[3].target), (5, &outline[5].target)]
  );
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
