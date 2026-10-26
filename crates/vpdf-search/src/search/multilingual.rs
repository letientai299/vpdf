use super::tests::sample;
use super::*;

fn restored(index: &Index) -> Result<Index> {
  let mut bytes = Vec::new();
  index.encode(&mut bytes);
  Index::decode(index.page, &bytes)
}

#[test]
fn vietnamese_literals_preserve_spans() -> Result<()> {
  let cases = [
    ("Tiếng Việt", "tieng viet"),
    ("Tie\u{302}\u{301}ng Vie\u{323}\u{302}t", "tieng viet"),
    ("Tiếng Việt", "tiếng việt"),
    ("điện đường", "dien duong"),
    ("ĐIỆN ĐƯỜNG", "DIEN DUONG"),
    ("a\u{302}\u{323}", "a"),
  ];
  for (source, query) in cases {
    let pattern = compile(query, false)?;
    assert_eq!(pattern.ranges(source), [(0, source.len())], "{source}");
    let index = Index::new(PageId(4), &sample(&[source]));
    for index in [index, restored(&Index::new(PageId(4), &sample(&[source])))?]
    {
      let hits = index.matches(&pattern, 10)?;
      assert_eq!(hits.len(), 1, "{source}");
      assert_eq!(hits[0].first.glyph, 0);
      assert_eq!(hits[0].last.glyph, source.chars().count() - 1);
      assert_eq!(hits[0].excerpt.trim_end(), source);
    }
  }
  assert_eq!(
    compile(r"\vtiếng", false)?.ranges("tieng"),
    [] as [(usize, usize); 0]
  );
  assert_eq!(compile(r"\vtiếng", false)?.ranges("Tiếng"), [(0, 7)]);
  Ok(())
}

#[test]
fn canonical_sequences_keep_coverage() -> Result<()> {
  for source in ["q\u{301}\u{323}", "q\u{323}\u{301}"] {
    for query in ["q\u{301}\u{323}", "q\u{323}\u{301}"] {
      let pattern = compile(query, false)?;
      assert_eq!(pattern.ranges(source), [(0, source.len())]);
      let index = restored(&Index::new(PageId(0), &sample(&[source])))?;
      let hits = index.matches(&pattern, 10)?;
      assert_eq!(hits.len(), 1);
      assert_eq!((hits[0].first.glyph, hits[0].last.glyph), (0, 2));
    }
  }
  for (source, query) in [("ガナ", "カナ"), ("كَت", "كت"), ("ñx", "nx")]
  {
    assert_eq!(
      compile(query, false)?.ranges(source),
      [] as [(usize, usize); 0]
    );
  }
  assert_eq!(compile("a", false)?.ranges("ab"), [(0, 1)]);
  Ok(())
}

fn wrapped() -> Text {
  let mut text = sample(&["中文排", "版需求"]);
  for (line, row) in text.lines.iter_mut().enumerate() {
    row.paragraph = line == 0;
    for (at, glyph) in row.glyphs.iter_mut().enumerate() {
      glyph.bounds = crate::text::Bounds {
        x0: at as f32 * 10.0,
        x1: (at + 1) as f32 * 10.0,
        y0: line as f32 * 16.0,
        y1: line as f32 * 16.0 + 12.0,
      };
    }
  }
  text
}

#[test]
fn cjk_continuations_survive_cache() -> Result<()> {
  let pattern = compile("排版", false)?;
  let index = Index::new(PageId(0), &wrapped());
  for index in [index, restored(&Index::new(PageId(0), &wrapped()))?] {
    let hits = index.matches(&pattern, 10)?;
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].first.line, hits[0].last.line), (0, 1));
    let from = hits[0].matched.start;
    assert_eq!(hits[0].excerpt[from..].trim_end(), "排版需求");
    let exact = compile(r"\v排版", false)?;
    assert_eq!(index.matches(&exact, 10)?.len(), 1);
  }
  for separate in 0..4 {
    let mut text = wrapped();
    match separate {
      0 => text.lines[1].paragraph = true,
      1 => {
        for glyph in &mut text.lines[1].glyphs {
          glyph.bounds.x0 += 200.0;
          glyph.bounds.x1 += 200.0;
        }
      }
      2 => {
        for glyph in &mut text.lines[1].glyphs {
          glyph.bounds.y0 += 200.0;
          glyph.bounds.y1 += 200.0;
        }
      }
      _ => {
        for glyph in &mut text.lines[1].glyphs {
          glyph.bounds.y0 -= 16.0;
          glyph.bounds.y1 -= 16.0;
        }
      }
    }
    assert!(Index::new(PageId(0), &text).matches(&pattern, 10)?.is_empty());
  }
  Ok(())
}

#[test]
fn dictionary_words_are_lazy() -> Result<()> {
  let text = sample(&["中文排版需求 こんにちは世界 東京 AI"]);
  let index = Index::new(PageId(0), &text);
  for (query, expected) in
    [("排版", 1), ("こんにちは", 1), ("京", 0), ("AI", 1)]
  {
    let plain = compile(query, false)?;
    index.matches(&plain, 10)?;
    if query == "排版" {
      assert!(index.words.get().is_none());
    }
    let pattern = compile(query, true)?;
    assert_eq!(index.matches(&pattern, 10)?.len(), expected, "{query}");
    assert_eq!(pattern.ranges(&text.lines[0].text()).len(), expected);
    assert_eq!(restored(&index)?.matches(&pattern, 10)?.len(), expected);
  }
  assert!(index.words.get().is_some_and(Option::is_some));
  let before = index.weight();
  index.matches(&compile("世界", true)?, 10)?;
  assert_eq!(index.weight(), before);
  let english = Index::new(PageId(0), &sample(&["the other"]));
  assert_eq!(english.matches(&compile("the", true)?, 10)?.len(), 1);
  assert!(english.words.get().is_some_and(Option::is_none));
  assert_eq!(index.matches(&compile(r"\vこ|こんにちは", true)?, 10)?.len(), 1);
  Ok(())
}

#[test]
fn normalization_matches_library_sequences() {
  use unicode_casefold::UnicodeCaseFold;
  use unicode_normalization::UnicodeNormalization;
  let source: String = (0..=0x0010_ffff).filter_map(char::from_u32).collect();
  let expected: String = source.nfkd().case_fold().nfkd().collect();
  let actual = normalized(&source, Form::RegexFolded);
  assert_eq!(actual.len(), expected.len());
  assert!(actual.chars().eq(expected.chars()));
  for source in ["α\u{345}\u{323}", "α\u{323}\u{345}", "\u{f73}\u{300}"] {
    let expected: String = source.nfkd().case_fold().nfkd().collect();
    assert_eq!(normalized(source, Form::RegexFolded), expected);
  }
  let source = format!("q{}", "\u{315}\u{300}\u{323}".repeat(100));
  let expected: String = source.nfkd().case_fold().nfkd().collect();
  assert_eq!(normalized(&source, Form::RegexFolded), expected);
}

#[test]
fn active_words_use_dictionary_boundaries() {
  for (source, at, expected) in [
    ("the other", 1, "the"),
    ("中文排版需求", 2, "排版"),
    ("こんにちは世界", 5, "世界"),
    ("Tiếng Việt", 7, "Việt"),
    ("Tie\u{302}\u{301}ng", 3, "Tie\u{302}\u{301}ng"),
  ] {
    let text = sample(&[source]);
    assert_eq!(word_at(&text.lines[0], at).as_deref(), Some(expected));
  }
  assert!(word_at(&sample(&["a b"]).lines[0], 1).is_none());
}

#[test]
fn whole_regex_scales_to_document_pages() -> Result<()> {
  let source = "中文排版需求。".repeat(500);
  let pattern = compile(r"\v排|排版", true)?;
  let index = Index::new(PageId(0), &sample(&[&source]));
  assert_eq!(index.matches(&pattern, 1000)?.len(), 500);
  assert_eq!(pattern.ranges(&source).len(), 500);
  Ok(())
}

#[test]
fn cjk_wraps_preserve_voicing_marks() -> Result<()> {
  for (base, mark) in [('ガ', None), ('カ', Some('\u{3099}')), ('ｶ', Some('ﾞ'))]
  {
    let mut text = wrapped();
    text.lines[0].glyphs[2].ch = base;
    if let Some(ch) = mark {
      let mut glyph = text.lines[0].glyphs[2].clone();
      glyph.ch = ch;
      text.lines[0].glyphs.push(glyph);
    }
    text.lines[1].glyphs[0].ch = 'ナ';
    let index = Index::new(PageId(0), &text);
    for query in ["ガナ", r"\vガナ"] {
      let pattern = compile(query, false)?;
      assert_eq!(index.matches(&pattern, 10)?.len(), 1, "{base}{mark:?}");
      assert_eq!(restored(&index)?.matches(&pattern, 10)?.len(), 1);
    }
  }
  Ok(())
}
