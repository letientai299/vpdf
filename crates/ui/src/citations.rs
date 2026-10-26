use crate::text::{PageId, Position, Text};
use vpdf_model::document::{Heading, Link, Target};
use vpdf_reader::reader::{JumpDescription, TextSource};

/// Whether `anchor` cites what `context` starts: a number, a bracket,
/// or the author the entry starts with.
fn cites(anchor: &str, context: &str) -> bool {
  let numeric = anchor.chars().filter(|ch| ch.is_alphabetic()).take(3).count();
  numeric < 3
    || anchor.starts_with('[') && anchor.ends_with(']')
    || leads(anchor, context)
}

/// Whether both start with the same word.
fn leads(anchor: &str, context: &str) -> bool {
  let word = |text: &str| -> String {
    text.chars().take_while(|ch| ch.is_alphabetic()).collect()
  };
  let first = word(anchor);
  !first.is_empty() && first == word(context)
}

fn excerpt(text: &Text, target: &Target) -> String {
  let start = match target {
    Target::Point { page, x, y } => {
      below(text, *x, *y).unwrap_or_else(|| text.nearest(*page, *x, *y).line)
    }
    _ => 0,
  };
  let context: String = text
    .lines
    .iter()
    .skip(start)
    .take(3)
    .enumerate()
    .take_while(|(index, line)| *index == 0 || !line.paragraph)
    .flat_map(|(_, line)| line.glyphs.iter().map(|glyph| glyph.ch).chain([' ']))
    .take(240)
    .collect();
  context.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The line nearest a point, among those not mostly above it:
/// reference targets sit in the gap above their entry, often nearer
/// the heading over the first one.
fn below(text: &Text, x: f32, y: f32) -> Option<usize> {
  let lines = text.lines.iter().enumerate();
  let scored = lines.filter_map(|(index, line)| {
    // Anchors can sit inside the line above, not past its middle.
    let span = line.span().filter(|span| span.y0 + span.y1 >= 2.0 * y)?;
    let dx = (span.x0 - x).max(0.0).max(x - span.x1);
    let dy = (span.y0 - y).max(0.0);
    Some((index, dx * dx + 4.0 * dy * dy))
  });
  scored.min_by(|a, b| a.1.total_cmp(&b.1)).map(|(index, _)| index)
}

/// A reference's anchor joined with the `context` it cites; empty
/// without context.
pub fn cited(anchor: &str, context: String) -> String {
  if context.is_empty() {
    return context;
  }
  if anchor.is_empty()
    || context.starts_with(&format!("[{anchor}]"))
    || leads(anchor, &context)
  {
    context
  } else if let Some(rest) = context
    .strip_prefix(anchor)
    .filter(|rest| !rest.starts_with(char::is_numeric))
  {
    format!("{anchor} {}", rest.trim_start()).trim_end().to_owned()
  } else {
    format!("{anchor} {context}").trim().to_owned()
  }
}

/// `→ p. 43 · Section`, `cited` in place of the section, or the
/// address.
pub fn destination<'a>(
  target: &Target,
  outline: &[Heading],
  pages: usize,
  name: impl Fn(PageId) -> &'a str,
  cited: &str,
) -> String {
  let (page, y) = match target {
    Target::Uri(uri) => return format!("→ {uri}"),
    Target::Page(page) => (*page, first_heading(outline, *page)),
    Target::Point { page, y, .. } => (*page, *y),
  };
  let page = PageId(page.0.min(pages.saturating_sub(1)));
  let name = name(page);
  if !cited.is_empty() {
    return format!("→ p. {name} · {cited}");
  }
  match vpdf_model::document::section(outline, page, y) {
    Some(entry) => format!("→ p. {name} · {}", outline[entry].title.trim()),
    None => format!("→ p. {name}"),
  }
}

/// First heading top on `page`, else 0.
fn first_heading(outline: &[Heading], page: PageId) -> f32 {
  let tops = outline.iter().filter_map(|heading| match heading.target {
    Target::Point { page: on, y, .. } if on == page => Some(y),
    _ => None,
  });
  tops.reduce(f32::min).unwrap_or(0.0)
}

/// A link's text on `page`, and for references, the text they cite.
pub fn context(
  page: PageId,
  link: &Link,
  text: Option<&Text>,
  source: &mut impl TextSource,
) -> (String, String) {
  let anchor = text.map_or_else(String::new, |text| link.anchor(text, page));
  let context = match link.target {
    Target::Page(page) | Target::Point { page, .. } => source
      .page_text(page)
      .ok()
      .map_or_else(String::new, |text| excerpt(&text, &link.target)),
    Target::Uri(_) => String::new(),
  };
  if !cites(&anchor, &context) {
    return (anchor, String::new());
  }
  (anchor, context)
}

pub fn description(
  target: &Target,
  position: Position,
  text: Option<&Text>,
  outline: &[Heading],
) -> JumpDescription {
  let Some(text) = text else {
    return JumpDescription::Link(String::new());
  };
  for heading in outline {
    let at = match heading.target {
      Target::Point { page, x, y } if page == position.page => {
        text.nearest(page, x, y)
      }
      Target::Page(page) if page == position.page => {
        Position { page, ..Position::default() }
      }
      _ => continue,
    };
    if at == position && !heading.title.trim().is_empty() {
      return JumpDescription::Heading(heading.title.trim().into());
    }
  }
  JumpDescription::Link(excerpt(text, target))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::Result;
  use crate::text::{Bounds, Glyph, Line as TextLine};
  #[test]
  fn failed_context_keeps_anchor() {
    struct Missing {
      reads: usize,
    }
    impl TextSource for Missing {
      fn page_count(&self) -> usize {
        1
      }
      fn page_text(&mut self, _: PageId) -> Result<std::sync::Arc<Text>> {
        self.reads += 1;
        anyhow::bail!("Text unavailable")
      }
    }
    let bounds = Bounds { x0: 0.0, y0: 0.0, x1: 10.0, y1: 10.0 };
    let text = Text {
      lines: vec![TextLine {
        ocr: None,
        paragraph: false,
        glyphs: vec![Glyph { quad: None, ch: '1', bounds }],
      }],
      truncated: false,
    };
    let mut source = Missing { reads: 0 };
    let page = PageId(0);
    let mut link = Link::new(bounds, Target::Page(page));
    assert_eq!(
      context(page, &link, Some(&text), &mut source),
      ("1".into(), String::new())
    );
    assert_eq!(source.reads, 1);
    link.target = Target::Uri("https://example.org".into());
    assert_eq!(
      context(page, &link, Some(&text), &mut source),
      ("1".into(), String::new())
    );
    assert_eq!(source.reads, 1);
  }

  #[test]
  fn cites_numbers_brackets_and_authors() {
    for (anchor, expected) in [
      ("[12]", true),
      ("*", true),
      ("a", true),
      ("[Smith, 2020]", true),
      ("Introduction", false),
    ] {
      assert_eq!(cites(anchor, "Unrelated text"), expected);
    }
  }

  #[test]
  fn excerpts_follow_destinations() {
    let text = Text {
      lines: [
        "Header",
        "† Equal contribution",
        "Authors listed together",
        "Unrelated paragraph",
      ]
      .into_iter()
      .enumerate()
      .map(|(index, value)| TextLine {
        ocr: None,
        paragraph: index != 2,
        glyphs: value
          .chars()
          .enumerate()
          .map(|(glyph, ch)| Glyph {
            quad: None,
            ch,
            bounds: Bounds {
              x0: glyph as f32 * 10.0,
              y0: index as f32 * 20.0,
              x1: (glyph + 1) as f32 * 10.0,
              y1: index as f32 * 20.0 + 10.0,
            },
          })
          .collect(),
      })
      .collect(),
      truncated: false,
    };
    let target = Target::Point { page: PageId(0), x: 0.0, y: 20.0 };
    assert_eq!(
      excerpt(&text, &target),
      "† Equal contribution Authors listed together"
    );
    assert_eq!(excerpt(&Text::default(), &target), "");
    // An anchor inside the line above starts below it.
    let inside = Target::Point { page: PageId(0), x: 0.0, y: 8.0 };
    assert_eq!(
      excerpt(&text, &inside),
      "† Equal contribution Authors listed together"
    );
  }

  /// U21: numbers, brackets, and leading authors quote their entry.
  #[test]
  fn citations_quote_their_entry() {
    let vaswani = "Vaswani, A., Shazeer, N. (2017). Attention is all you need.";
    // (anchor, excerpt) -> label text, or None for the section
    let cases = [
      ("13", "[13] Sepp Hochreiter.", Some("[13] Sepp Hochreiter.")),
      ("[2]", "Jimmy Ba.", Some("[2] Jimmy Ba.")),
      ("[12]", "Attention", Some("[12] Attention")),
      ("12", "[12] Reference title", Some("[12] Reference title")),
      ("†", "Equal contribution", Some("† Equal contribution")),
      ("4", "4To illustrate", Some("4 To illustrate")),
      ("2", "2 Code is online", Some("2 Code is online")),
      ("", "Introduction", Some("Introduction")),
      ("Vaswani et al., 2017", vaswani, Some(vaswani)),
      ("Section 3", "3 Model Architecture", None),
      ("See the results", "Results heading", None),
      (
        "Ba",
        "Ba, J. Layer normalization.",
        Some("Ba, J. Layer normalization."),
      ),
    ];
    for (anchor, context, label) in cases {
      let quoted =
        cites(anchor, context).then(|| cited(anchor, context.to_owned()));
      assert_eq!(quoted.as_deref(), label, "{anchor}");
    }
  }
}
