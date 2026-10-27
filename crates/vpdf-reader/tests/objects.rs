#[path = "../../../tests/support/text.rs"]
mod text_fixture;
use std::sync::Arc;
use text_fixture::{plain, text};
use vpdf_model::text::{Line, PageId, Position, Text};
use vpdf_reader::Result;
use vpdf_reader::motion::Motion;
use vpdf_reader::objects::Object;
use vpdf_reader::reader::{Reader, SelectionKind, TextSource};

type Pages = Vec<Arc<Text>>;

fn pages(pages: &[&[(&str, bool)]]) -> Pages {
  pages
    .iter()
    .map(|lines| {
      Arc::new(text(
        lines
          .iter()
          .map(|&(value, paragraph)| Line { paragraph, ..plain(value) }),
      ))
    })
    .collect()
}

fn copy(
  source: &mut Pages,
  cursor: Position,
  kind: Object,
  around: bool,
  count: usize,
) -> Result<String> {
  let mut reader = Reader::default();
  reader.cursor = cursor;
  reader.yank(1, 'a', source)?;
  reader.select_object(kind, around, count, source)?;
  Ok(
    reader
      .pending_yank
      .take()
      .unwrap_or_else(|| {
        panic!(
          "Missing {kind:?} around={around} count={count} cursor={cursor:?}"
        )
      })
      .text,
  )
}

#[test]
fn object_words_and_counts() -> Result<()> {
  let mut source = pages(&[
    &[("α-beta,  next", true), ("last", false)],
    &[],
    &[("page", true)],
  ]);
  let cursor = Position { glyph: 3, ..Position::default() };
  for (kind, around, count, want) in [
    (Object::Word, false, 1, "beta"),
    (Object::BigWord, false, 1, "α-beta,"),
    (Object::BigWord, true, 1, "α-beta,"),
    (Object::BigWord, false, 3, "α-beta,  next\nlast"),
    (Object::BigWord, false, 4, "α-beta,  next\nlast\npage"),
  ] {
    assert_eq!(copy(&mut source, cursor, kind, around, count)?, want);
  }
  let mut reader = Reader::default();
  reader.yank(2, 'b', &mut source)?;
  reader.select_object(Object::BigWord, false, 2, &mut source)?;
  assert_eq!(
    reader.pending_yank.take().unwrap().text,
    "α-beta,  next\nlast\npage"
  );
  assert_eq!(reader.cursor, Position::default());
  let cursor = Position { page: PageId(2), ..Position::default() };
  assert_eq!(copy(&mut source, cursor, Object::BigWord, true, 1)?, "page");
  Ok(())
}

#[test]
fn object_pairs_nest_and_cross_pages() -> Result<()> {
  for (kind, open, close) in [
    (Object::Parentheses, '(', ')'),
    (Object::Brackets, '[', ']'),
    (Object::Braces, '{', '}'),
    (Object::Angles, '<', '>'),
  ] {
    let left = format!("{open}before {open}inner");
    let right = format!("text{close} after{close}");
    let mut source = pages(&[&[(&left, true)], &[], &[(&right, true)]]);
    let cursor = Position { glyph: 10, ..Position::default() };
    assert_eq!(copy(&mut source, cursor, kind, false, 1)?, "inner\ntext");
    assert_eq!(
      copy(&mut source, cursor, kind, true, 1)?,
      format!("{open}inner\ntext{close}")
    );
    assert_eq!(
      copy(&mut source, cursor, kind, false, 2)?,
      format!("before {open}inner\ntext{close} after")
    );
    let cursor = Position { page: PageId(2), glyph: 4, ..Position::default() };
    assert_eq!(copy(&mut source, cursor, kind, false, 1)?, "inner\ntext");
    let cursor = Position { page: PageId(2), glyph: 7, ..Position::default() };
    assert_eq!(
      copy(&mut source, cursor, kind, false, 1)?,
      format!("before {open}inner\ntext{close} after")
    );
  }
  Ok(())
}

#[test]
fn object_quotes_and_apostrophes() -> Result<()> {
  for (kind, open, close) in [
    (Object::DoubleQuote, '"', '"'),
    (Object::DoubleQuote, '“', '”'),
    (Object::SingleQuote, '\'', '\''),
    (Object::SingleQuote, '‘', '’'),
  ] {
    let first = format!("don't {open}quoted");
    let last = format!("text{close} then {open}again{close}");
    let mut source = pages(&[&[(&first, true)], &[], &[(&last, true)]]);
    let cursor = Position { glyph: 9, ..Position::default() };
    assert_eq!(copy(&mut source, cursor, kind, false, 1)?, "quoted\ntext");
    assert_eq!(
      copy(&mut source, cursor, kind, true, 1)?,
      format!("{open}quoted\ntext{close}")
    );
    assert_eq!(
      copy(&mut source, cursor, kind, false, 2)?,
      format!("quoted\ntext{close} then {open}again")
    );
    let cursor = Position { page: PageId(2), glyph: 4, ..Position::default() };
    assert_eq!(copy(&mut source, cursor, kind, false, 1)?, "quoted\ntext");
  }
  let mut source = pages(&[&[(r#""escaped \" quote" end"#, true)]]);
  assert_eq!(
    copy(
      &mut source,
      Position { glyph: 3, ..Position::default() },
      Object::DoubleQuote,
      false,
      1
    )?,
    r#"escaped \" quote"#
  );
  Ok(())
}

#[test]
fn object_quote_delimiter_edges() -> Result<()> {
  for text in [r#"" hello " outside"#, r#"before,"hello" after"#, r#""hello""#]
  {
    let mut source = pages(&[&[(text, true)]]);
    let first = text.chars().position(|ch| ch == '"').unwrap();
    let last = text
      .chars()
      .collect::<Vec<_>>()
      .iter()
      .rposition(|&ch| ch == '"')
      .unwrap();
    let expected: String =
      text.chars().skip(first + 1).take(last - first - 1).collect();
    for glyph in [first, first + 1, last] {
      assert_eq!(
        copy(
          &mut source,
          Position { glyph, ..Position::default() },
          Object::DoubleQuote,
          false,
          1
        )?,
        expected.trim()
      );
    }
  }
  Ok(())
}

#[test]
fn object_sentences_skip_scientific_abbreviations() -> Result<()> {
  let mut source = pages(&[
    &[
      ("See Fig. 2 and et al. for e.g. details.", true),
      ("Next sentence", false),
    ],
    &[],
    &[("ends here! Last one?", false)],
  ]);
  let cursor = Position { glyph: 15, ..Position::default() };
  assert_eq!(
    copy(&mut source, cursor, Object::Sentence, false, 1)?,
    "See Fig. 2 and et al. for e.g. details."
  );
  assert_eq!(
    copy(&mut source, cursor, Object::Sentence, false, 2)?,
    "See Fig. 2 and et al. for e.g. details.\nNext sentence\nends here!"
  );
  let mut reader = Reader::default();
  reader.motion(Motion::NextSentence, 1, false, &mut source)?;
  assert_eq!(reader.cursor, Position { line: 1, ..Position::default() });
  reader.motion(Motion::NextSentence, 1, false, &mut source)?;
  assert_eq!(
    reader.cursor,
    Position { page: PageId(2), glyph: 11, ..Position::default() }
  );
  reader.motion(Motion::PreviousSentence, 2, false, &mut source)?;
  assert_eq!(reader.cursor, Position::default());
  reader.yank(1, 'a', &mut source)?;
  reader.motion(Motion::NextSentence, 10, false, &mut source)?;
  assert_eq!(
    reader.pending_yank.take().unwrap().text,
    concat!(
      "See Fig. 2 and et al. for e.g. details.\n",
      "Next sentence\nends here! Last one?"
    )
  );
  Ok(())
}

#[test]
fn object_failures_preserve_visual_range() -> Result<()> {
  let mut source = pages(&[&[("unpaired (text", true)]]);
  let mut reader = Reader::default();
  reader.visual(SelectionKind::Line);
  reader.cursor.glyph = 11;
  let old = reader.cursor;
  for kind in [Object::Parentheses, Object::DoubleQuote, Object::Brackets] {
    reader.select_object(kind, false, 1, &mut source)?;
    assert_eq!(reader.cursor, old);
    assert_eq!(reader.selection.unwrap().anchor, Position::default());
    assert_eq!(reader.selection.unwrap().kind, SelectionKind::Line);
  }
  reader.cancel();
  reader.yank(1, 'a', &mut source)?;
  reader.select_object(Object::Parentheses, false, 1, &mut source)?;
  assert!(reader.operator.is_none() && reader.pending_yank.is_none());
  Ok(())
}

#[test]
fn object_word_motions_cross_lines_and_blank_pages() -> Result<()> {
  let mut source = pages(&[
    &[("one-two, next", true), ("last", false)],
    &[],
    &[("page", true)],
  ]);
  let mut reader = Reader::default();
  for (motion, page, line, glyph) in [
    (Motion::BigWord, 0, 0, 9),
    (Motion::EndBigWord, 0, 0, 12),
    (Motion::BigWord, 0, 1, 0),
    (Motion::BigWord, 2, 0, 0),
    (Motion::BackBigWord, 0, 1, 0),
    (Motion::PreviousBigEnd, 0, 0, 12),
    (Motion::BackBigWord, 0, 0, 9),
    (Motion::PreviousBigEnd, 0, 0, 7),
    (Motion::EndBigWord, 0, 0, 12),
  ] {
    reader.motion(motion, 1, false, &mut source)?;
    assert_eq!(
      reader.cursor,
      Position { page: PageId(page), line, glyph },
      "{motion:?}"
    );
  }
  reader.cursor = Position { page: PageId(2), ..Position::default() };
  reader.yank(1, 'a', &mut source)?;
  reader.motion(Motion::BigWord, 1, false, &mut source)?;
  assert_eq!(reader.pending_yank.take().unwrap().text, "page");
  Ok(())
}

/// Pages, then copies: cursor (page, line, glyph), object, around,
/// count, and the copied text.
type Copies<'a> =
  (&'a str, Pages, &'a [((usize, usize, usize), Object, bool, usize, &'a str)]);

#[test]
fn object_copies() -> Result<()> {
  use Object::{DoubleQuote as Double, Paragraph, Sentence, SingleQuote};
  let wrapped = r#""wrapped"#;
  let cases: [Copies; 8] = [
    (
      "paragraphs follow blocks",
      pages(&[
        &[("first", true), ("wrapped", false), ("second", true)],
        &[],
        &[("continued", false), ("third", true)],
      ]),
      &[
        ((0, 1, 3), Paragraph, false, 1, "first\nwrapped"),
        ((0, 1, 3), Paragraph, true, 1, "first\nwrapped"),
        ((0, 1, 3), Paragraph, false, 2, "first\nwrapped\nsecond\ncontinued"),
        ((2, 0, 2), Paragraph, false, 1, "second\ncontinued"),
      ],
    ),
    (
      "quotes keep stream context",
      pages(&[&[(r#""hello ", then "world""#, true)]]),
      &[
        ((0, 0, 0), Double, false, 1, "hello"),
        ((0, 0, 3), Double, false, 1, "hello"),
        ((0, 0, 7), Double, false, 1, "hello"),
        ((0, 0, 15), Double, false, 1, "world"),
        ((0, 0, 18), Double, false, 1, "world"),
        ((0, 0, 20), Double, false, 1, "world"),
      ],
    ),
    (
      "quotes wrap across pages",
      pages(&[&[(wrapped, true)], &[], &[(r#"text ", then "next""#, true)]]),
      &[
        ((2, 0, 5), Double, false, 1, "wrapped\ntext"),
        ((2, 0, 5), Double, false, 2, "wrapped\ntext \", then \"next"),
      ],
    ),
    (
      "quotes close at page start",
      pages(&[&[(wrapped, true)], &[], &[(r#"" then "next""#, true)]]),
      &[((2, 0, 0), Double, false, 1, "wrapped")],
    ),
    (
      "sentences end at the final glyph",
      pages(&[&[("First. X", true)]]),
      &[
        ((0, 0, 0), Sentence, false, 2, "First. X"),
        ((0, 0, 0), Sentence, false, 3, "First. X"),
        ((0, 0, 0), Sentence, false, 100, "First. X"),
      ],
    ),
    (
      "sentences skip indentation",
      pages(&[&[("Methods", true), ("  Results follow.", true)]]),
      &[((0, 1, 5), Sentence, false, 1, "Results follow.")],
    ),
    (
      "wrapped quote before dash",
      pages(&[&[(wrapped, true), ("text\"—then \"next\"", true)]]),
      &[
        ((0, 1, 1), Double, false, 1, "wrapped\ntext"),
        ((0, 1, 1), Double, true, 1, "\"wrapped\ntext\""),
        ((0, 1, 4), Double, false, 1, "wrapped\ntext"),
        ((0, 1, 4), Double, true, 1, "\"wrapped\ntext\""),
      ],
    ),
    (
      "single quotes after possessive",
      pages(&[&[("The dogs' owners said 'hello'.", true)]]),
      &[
        ((0, 0, 22), SingleQuote, false, 1, "hello"),
        ((0, 0, 22), SingleQuote, true, 1, "'hello'"),
        ((0, 0, 25), SingleQuote, false, 1, "hello"),
        ((0, 0, 25), SingleQuote, true, 1, "'hello'"),
        ((0, 0, 28), SingleQuote, false, 1, "hello"),
        ((0, 0, 28), SingleQuote, true, 1, "'hello'"),
      ],
    ),
  ];
  for (label, mut source, copies) in cases {
    for &((page, line, glyph), kind, around, count, want) in copies {
      let cursor = Position { page: PageId(page), line, glyph };
      let copied = copy(&mut source, cursor, kind, around, count)?;
      assert_eq!(copied, want, "{label} {kind:?} {cursor:?}");
    }
  }
  Ok(())
}

#[test]
fn object_sentence_motions_skip_indentation() -> Result<()> {
  let mut source = pages(&[&[("Methods", true), ("  Results follow.", true)]]);
  let mut reader = Reader::default();
  reader.motion(Motion::NextSentence, 1, false, &mut source)?;
  assert_eq!(
    reader.cursor,
    Position { line: 1, glyph: 2, ..Position::default() }
  );
  reader.motion(Motion::PreviousSentence, 1, false, &mut source)?;
  assert_eq!(reader.cursor, Position::default());
  Ok(())
}

struct LocalSource {
  pages: Pages,
  page: PageId,
}

impl TextSource for LocalSource {
  fn page_count(&self) -> usize {
    self.pages.page_count()
  }
  fn page_text(&mut self, page: PageId) -> Result<Arc<Text>> {
    assert_eq!(page, self.page, "Local quote fetched another page");
    self.pages.page_text(page)
  }
}

#[test]
fn object_local_quotes_avoid_previous_pages() -> Result<()> {
  for (kind, text) in
    [(Object::DoubleQuote, "\"hello\""), (Object::SingleQuote, "'hello'")]
  {
    let mut source = LocalSource {
      pages: pages(&[
        &[("earlier", true)],
        &[("earlier", true)],
        &[(text, true)],
      ]),
      page: PageId(2),
    };
    for glyph in [0, 3, 6] {
      let mut reader = Reader::default();
      reader.cursor =
        Position { page: PageId(2), glyph, ..Position::default() };
      reader.yank(1, 'a', &mut source)?;
      reader.select_object(kind, false, 1, &mut source)?;
      assert_eq!(reader.pending_yank.take().unwrap().text, "hello");
    }
  }
  Ok(())
}
