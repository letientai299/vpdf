//! Caption labels such as `Figure 3:` and `Table 2.`

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
  Figure,
  Table,
}

/// A caption's label and the text after it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Caption<'a> {
  pub kind: Kind,
  /// `3`, `2.4` or `12(a)`.
  pub number: &'a str,
  pub rest: &'a str,
}

/// The label a caption word names.
pub fn kind(word: &str) -> Option<Kind> {
  if word.eq_ignore_ascii_case("figure") {
    Some(Kind::Figure)
  } else if word.eq_ignore_ascii_case("table") {
    Some(Kind::Table)
  } else {
    None
  }
}

/// Reads `Figure 2.4: A plot`; body text such as `Figure 2 shows` or
/// `Table 2 summarizes` is not a caption.
pub fn parse(text: &str) -> Option<Caption<'_>> {
  let text = text.trim_start();
  let (prefix, after) = text.split_once(char::is_whitespace)?;
  let kind = kind(prefix)?;
  let after = after.trim_start();
  let (number, rest) =
    after.split_once(char::is_whitespace).unwrap_or((after, ""));
  let rest = rest.trim_start();
  let standalone = rest.is_empty();
  let separated = number.ends_with([':', '.']);
  let bare = number.trim_end_matches([':', '.']);
  let lettered = standalone
    && number.ends_with(')')
    && number.split_once('(').is_some_and(|(base, _)| numbered(base));
  // A capitalized title may follow without a separator.
  let titled = rest.starts_with(char::is_uppercase);
  let labeled = numbered(bare) && (standalone || separated || titled);
  (lettered || labeled).then(|| Caption {
    kind,
    number: if lettered { number } else { bare },
    rest: rest.trim_start_matches([':', '.', '—', '–', '-', ' ']),
  })
}

fn numbered(value: &str) -> bool {
  value.starts_with(|c: char| c.is_ascii_digit())
    && value.ends_with(|c: char| c.is_ascii_digit())
    && value.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-')
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn captions_parse_their_labels() {
    use Kind::{Figure as F, Table as T};
    // (text, kind, number, rest)
    let cases = [
      ("Figure 12(a)", Some((F, "12(a)", ""))),
      ("table 2", Some((T, "2", ""))),
      ("Table 1 Costs here", Some((T, "1", "Costs here"))),
      ("Table 2. caption", Some((T, "2", "caption"))),
      ("Figure 2.4: A plot", Some((F, "2.4", "A plot"))),
      ("FIGURE 2.3 Execution", Some((F, "2.3", "Execution"))),
      ("Table 1 alone here", None),
      ("Table 2 summarizes our results", None),
      ("Figurehead 2", None),
      ("Figure text", None),
      ("Figure 2.6 shows some plots", None),
      ("Figure 2.10(a) gives an example", None),
      ("Figure 4.3b. This is called a network", None),
      ("Table 3.1).", None),
      ("Figure 3 (left) shows the loss", None),
      ("Table 4 [12] reports results", None),
      ("Figure 5 2D view", None),
    ];
    for (text, want) in cases {
      let got = parse(text).map(|c| (c.kind, c.number, c.rest));
      assert_eq!(got, want, "{text}");
    }
  }
}
