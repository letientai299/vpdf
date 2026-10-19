//! Finds web addresses, DOIs, and e-mail addresses printed as plain
//! text.
//!
//! Trailing punctuation follows the GFM autolink extension:
//! <https://github.github.com/gfm/#autolinks-extension->. DOIs follow
//! Crossref's pattern:
//! <https://www.crossref.org/blog/dois-and-matching-regular-expressions/>.

use crate::document::{Link, Target};
use crate::text::{Bounds, Text};
use std::ops::Range;
use std::ops::RangeInclusive;
use vpdf_model::reflow::{continuation, in_url, tight};

/// Printed addresses no link in `taken` covers.
pub fn find(text: &Text, taken: &[Link]) -> Vec<Link> {
  let chars: Vec<Vec<char>> = text
    .lines
    .iter()
    .map(|line| line.glyphs.iter().map(|glyph| glyph.ch).collect())
    .collect();
  let mut links = Vec::new();
  for found in scan(&chars, |line| continuation(text, line)) {
    let glyphs = found.pieces.iter().map(|(line, range)| {
      text.lines[*line].glyphs[range.clone()].iter().map(|glyph| glyph.bounds)
    });
    let covered = glyphs.clone().flatten().any(|bounds| {
      let (x, y) = bounds.centre();
      taken.iter().any(|link| link.contains(x, y))
    });
    if covered {
      continue;
    }
    let mut boxes = glyphs.filter_map(|piece| piece.reduce(Bounds::union));
    let Some(first) = boxes.next() else { continue };
    let mut link = Link::new(first, Target::Uri(found.target));
    boxes.for_each(|bounds| link.push(bounds));
    links.push(link);
  }
  links
}

/// A line and a character range on it.
type Piece = (usize, Range<usize>);

/// An address and the pieces of lines it spans.
#[derive(Debug, PartialEq)]
struct Found {
  target: String,
  pieces: Vec<Piece>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
  /// `http://` or `https://`, `scheme` characters long.
  Web { scheme: usize },
  /// `mailto:`.
  Mail,
  /// A bare address `len` characters long: gains `mailto:`.
  Email { len: usize },
  /// `www.`: gains `http://`.
  Www,
  /// `doi:10.…`, `doi.org/10.…`, or a bare `10.…`; `skip` drops the
  /// prefix. Printed `doi.org/` forms come escaped already.
  Doi { skip: usize, escape: bool },
}

impl Kind {
  /// Text put before the address, characters dropped from its front,
  /// and leading characters compared lowercase.
  fn parts(self) -> (&'static str, usize, usize) {
    match self {
      Self::Web { scheme } => ("", 0, scheme),
      Self::Mail => ("", 0, 7),
      Self::Email { .. } => ("mailto:", 0, 0),
      Self::Www => ("http://", 0, 0),
      Self::Doi { skip, .. } => ("https://doi.org/", skip, 0),
    }
  }
}

/// Addresses in `lines`; one ending at a separator goes on in the line
/// `wraps` names.
fn scan(
  lines: &[Vec<char>],
  wraps: impl Fn(usize) -> Option<usize>,
) -> Vec<Found> {
  let mut out = Vec::new();
  // Characters earlier addresses took from the front of each line.
  let mut taken = vec![0; lines.len()];
  let (mut line, mut at) = (0, 0);
  while line < lines.len() {
    let Some((start, kind)) = opening(&lines[line], at) else {
      line += 1;
      at = taken.get(line).copied().unwrap_or(0);
      continue;
    };
    if let Kind::Email { len } = kind
      && split_email(lines, &wraps, line, start..start + len)
    {
      at = start + 1;
      continue;
    }
    let mut pieces = Vec::new();
    let mut raw = Vec::new();
    let (mut row, mut from) = (line, start);
    loop {
      let chars = &lines[row];
      let end = match kind {
        Kind::Email { len } => from + len,
        _ => from + chars[from..].iter().take_while(|ch| part(**ch)).count(),
      };
      pieces.push((row, from..end));
      raw.extend_from_slice(&chars[from..end]);
      let ends = chars[end..].iter().all(|ch| ch.is_whitespace());
      let Some(next) =
        (ends && in_url(&target(kind, &raw))).then(|| wraps(row)).flatten()
      else {
        break;
      };
      let lead = lines[next].iter().take_while(|ch| ch.is_whitespace()).count();
      // A row opening its own scheme does not continue this address.
      let fresh = opening(&lines[next], lead).is_some_and(|(at, kind)| {
        at == lead
          && matches!(
            kind,
            Kind::Web { .. } | Kind::Www | Kind::Mail | Kind::Email { .. }
          )
      });
      if next <= row
        || fresh
        || !lines[next].get(lead).is_some_and(|ch| part(*ch))
      {
        break;
      }
      (row, from) = (next, lead);
    }
    raw.truncate(trimmed(&raw));
    if !valid(kind, &raw) {
      at = start + 1;
      continue;
    }
    at = pieces[0].1.end;
    for (row, range) in &pieces[1..] {
      taken[*row] = range.end;
    }
    cut(&mut pieces, raw.len());
    out.push(Found { target: target(kind, &raw), pieces });
  }
  out
}

/// Whether the bare address at `range` of `line` may be part of one
/// wrapped across rows, which the `url` package breaks after `.`, `-`,
/// and `@`: `john.` above `smith@…`, or `…@mail.google.` above `com`.
fn split_email(
  lines: &[Vec<char>],
  wraps: impl Fn(usize) -> Option<usize>,
  line: usize,
  range: Range<usize>,
) -> bool {
  let chars = &lines[line];
  let lead = chars.iter().take_while(|ch| ch.is_whitespace()).count();
  let broken = |row: &[char]| {
    let row: Vec<_> =
      row.iter().rev().skip_while(|ch| ch.is_whitespace()).collect();
    let mark = |ch: &char| matches!(ch, '.' | '-' | '_' | '+' | '@');
    matches!(row.as_slice(), [last, before, ..]
      if mark(last) && before.is_alphanumeric())
  };
  let above = range.start == lead
    && (line.saturating_sub(4)..line)
      .any(|row| wraps(row) == Some(line) && broken(&lines[row]));
  let below = match &chars[range.end..] {
    [] => false,
    [mark, rest @ ..] => {
      matches!(mark, '.' | '-')
        && rest.iter().all(|ch| ch.is_whitespace())
        && wraps(line).is_some()
    }
  };
  above || below
}

/// The first address start in `chars` at or after `at`.
fn opening(chars: &[char], at: usize) -> Option<(usize, Kind)> {
  let mail = chars[at..].contains(&'@');
  let email = |index: usize| {
    let edge = mail && email_edge(&chars[..index]);
    edge.then(|| email(&chars[index..])).flatten()
  };
  (at..chars.len()).find_map(|index| {
    let rest = &chars[index..];
    if index > 0 && glued(chars[index - 1]) {
      return Some((index, Kind::Email { len: email(index)? }));
    }
    let prefix = |word: &str| opens(rest, word);
    let kind = if prefix("http://") {
      Kind::Web { scheme: 7 }
    } else if prefix("https://") {
      Kind::Web { scheme: 8 }
    } else if prefix("mailto:") {
      Kind::Mail
    } else if prefix("www.") {
      Kind::Www
    } else {
      let doi = [("doi:", true), ("doi.org/", false), ("dx.doi.org/", false)];
      let doi = doi.into_iter().chain([("", true)]).find(|(word, _)| {
        prefix(word) && rest[word.len()..].starts_with(&['1', '0', '.'])
      });
      match doi {
        Some((word, escape)) => Kind::Doi { skip: word.len(), escape },
        None => Kind::Email { len: email(index)? },
      }
    };
    Some((index, kind))
  })
}

/// Whether a bare e-mail address may start after `before`: a word's
/// start, punctuation such as quotes or footnote marks, or the colon of
/// a `mail:` label; never inside a `user:pass@host` URL.
fn email_edge(before: &[char]) -> bool {
  match before {
    [] => true,
    [.., ch] if ch.is_whitespace() => true,
    [rest @ .., ':'] => {
      let label = |word: &str| {
        let word: Vec<char> = word.chars().collect();
        rest.len() >= word.len()
          && rest[rest.len() - word.len()..]
            .iter()
            .zip(&word)
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
          && !rest[..rest.len() - word.len()]
            .last()
            .is_some_and(|ch| ch.is_alphanumeric())
      };
      ["mail", "email", "e-mail"].into_iter().any(label)
    }
    [
      rest @ ..,
      '(' | '<' | '[' | '{' | '"' | '\'' | '`' | '\u{201C}' | '\u{2018}'
      | '\u{AB}',
    ] => !rest.last().is_some_and(|ch| ch.is_alphanumeric()),
    [.., ch] => {
      // RFC 5322 local-part specials continue an address.
      !ch.is_alphanumeric() && !"!#$%&*+-./=?@^_|~".contains(*ch)
    }
  }
}

/// Length of the bare e-mail address `chars` open with: an ASCII local
/// part, `@`, and two or more host labels ending in a one-case letters
/// top-level domain, then no more of a path, call, or word.
fn email(chars: &[char]) -> Option<usize> {
  let local = chars
    .iter()
    .take_while(|ch| {
      ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '+' | '-')
    })
    .count();
  if local == 0
    || chars[0] == '.'
    || chars[local - 1] == '.'
    || chars[..local].windows(2).any(|pair| pair == ['.', '.'])
    || chars.get(local) != Some(&'@')
  {
    return None;
  }
  let (mut at, mut end, mut labels, mut last) = (local + 1, 0, 0, 0..0);
  loop {
    let rest = chars.get(at..).unwrap_or_default();
    let len = rest
      .iter()
      .take_while(|ch| ch.is_ascii_alphanumeric() || **ch == '-')
      .count();
    if len == 0 || rest[0] == '-' || rest[len - 1] == '-' {
      break;
    }
    (end, labels, last) = (at + len, labels + 1, at..at + len);
    if chars.get(end) != Some(&'.') {
      break;
    }
    at = end + 1;
  }
  let top = &chars[last];
  let one_case = top.iter().all(char::is_ascii_lowercase)
    || top.iter().all(char::is_ascii_uppercase);
  // Prose punctuation may follow; `:org/repo`, `(…)`, `/path` may not.
  let closing = |ch: &char| {
    matches!(
      ch,
      ','
        | ';'
        | '!'
        | '?'
        | ')'
        | ']'
        | '}'
        | '>'
        | '"'
        | '\''
        | '\u{201D}'
        | '\u{2019}'
        | '\u{BB}'
        | '\u{2013}'
        | '\u{2014}'
        | '\u{2217}'
        | '*'
        | '\u{2020}'
        | '\u{2021}'
    )
  };
  let after = match chars.get(end..).unwrap_or_default() {
    [] | [':' | '.'] => true,
    [ch, ..] if ch.is_whitespace() => true,
    [':' | '.', next, ..] => next.is_whitespace() || closing(next),
    [ch, ..] => closing(ch),
  };
  // `icon@2x.png` names an image file.
  let file = ["png", "jpg", "jpeg", "gif", "svg", "webp", "bmp", "tif", "tiff"]
    .iter()
    .any(|extension| {
      top.iter().map(char::to_ascii_lowercase).eq(extension.chars())
    });
  (labels >= 2 && top.len() >= 2 && one_case && after && !file).then_some(end)
}

/// `chars` begin with lowercase `word`, ignoring ASCII case.
fn opens(chars: &[char], word: &str) -> bool {
  chars.len() >= word.len()
    && chars.iter().zip(word.chars()).all(|(a, b)| a.to_ascii_lowercase() == b)
}

/// Characters that make an address start mid-word.
fn glued(ch: char) -> bool {
  ch.is_alphanumeric() && !unspaced(ch)
    || matches!(ch, '/' | '.' | '-' | '_' | '@' | ':' | '%')
}

/// Hangul syllables and jamo, whose particles follow words unspaced.
const HANGUL: [RangeInclusive<char>; 3] =
  ['\u{1100}'..='\u{11FF}', '\u{3130}'..='\u{318F}', '\u{AC00}'..='\u{D7AF}'];

/// A letter of a script that can touch an address without a space.
fn unspaced(ch: char) -> bool {
  tight(ch) || HANGUL.iter().any(|range| range.contains(&ch))
}

/// A character an address can hold: RFC 3986's set plus letters and
/// typographic hyphens; not superscripts or unspaced scripts.
fn part(ch: char) -> bool {
  ch.is_alphabetic() && !unspaced(ch)
    || ch.is_ascii_digit()
    || matches!(ch, '\u{2010}' | '\u{2011}')
    || ch.is_ascii_punctuation() && !"\"<>\\^`{|}".contains(ch)
}

/// Length of `raw` without trailing punctuation or unmatched closers.
fn trimmed(raw: &[char]) -> usize {
  let mut end = raw.len();
  while let Some(&last) = raw[..end].last() {
    let count = |ch: char| raw[..end].iter().filter(|&&c| c == ch).count();
    let drop = match last {
      '.' | ',' | ':' | ';' | '!' | '?' | '*' | '\'' | '_' | '~' => true,
      ')' => count('(') < count(')'),
      ']' => count('[') < count(']'),
      _ => false,
    };
    if !drop {
      break;
    }
    end -= 1;
  }
  end
}

/// Whether `raw` holds an address of `kind` past its prefix. One ending
/// in a hyphen lost its wrapped rest.
fn valid(kind: Kind, raw: &[char]) -> bool {
  let after = |prefix: usize| raw.get(prefix..).unwrap_or_default();
  let whole = match kind {
    Kind::Web { scheme } => {
      after(scheme).first().is_some_and(|ch| ch.is_alphanumeric())
    }
    Kind::Email { .. } => true,
    Kind::Mail => {
      let address = after(7);
      address.iter().position(|ch| *ch == '@').is_some_and(|at| {
        at > 0 && address[at + 1..].iter().any(|ch| ch.is_alphanumeric())
      })
    }
    Kind::Www => {
      let host = after(4);
      host.first().is_some_and(|ch| ch.is_alphanumeric())
        && host.windows(2).any(|w| w[0] == '.' && w[1].is_alphanumeric())
    }
    Kind::Doi { skip, .. } => {
      let doi = after(skip + 3);
      let digits = doi.iter().take_while(|ch| ch.is_ascii_digit()).count();
      (4..=9).contains(&digits)
        && doi.get(digits) == Some(&'/')
        && doi.len() > digits + 1
    }
  };
  whole && !matches!(raw.last(), Some('-' | '\u{2010}' | '\u{2011}'))
}

/// Shortens `pieces` to `len` characters in all.
fn cut(pieces: &mut Vec<Piece>, len: usize) {
  let mut left = len;
  pieces.retain_mut(|(_, range)| {
    range.end = range.start + range.len().min(left);
    left -= range.len();
    range.start < range.end
  });
}

/// The address `raw` opens.
fn target(kind: Kind, raw: &[char]) -> String {
  let (head, skip, lower) = kind.parts();
  let mut out = head.to_owned();
  for (index, &ch) in raw.iter().enumerate().skip(skip) {
    match ch {
      ch if index < lower => out.push(ch.to_ascii_lowercase()),
      // DOI handbook: these are escaped in URL form.
      '#' | '?' | '%' if matches!(kind, Kind::Doi { escape: true, .. }) => {
        out.push_str(match ch {
          '#' => "%23",
          '?' => "%3F",
          _ => "%25",
        });
      }
      '\u{2010}' | '\u{2011}' => out.push('-'),
      '\u{FB00}' => out.push_str("ff"),
      '\u{FB01}' => out.push_str("fi"),
      '\u{FB02}' => out.push_str("fl"),
      '\u{FB03}' => out.push_str("ffi"),
      '\u{FB04}' => out.push_str("ffl"),
      '\u{FB05}' | '\u{FB06}' => out.push_str("st"),
      ch => out.push(ch),
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  fn chars(lines: &[&str]) -> Vec<Vec<char>> {
    lines.iter().map(|line| line.chars().collect()).collect()
  }

  /// Addresses in `lines`, each wrapping to the line `wraps` names.
  fn found_with(
    lines: &[&str],
    wraps: impl Fn(usize) -> Option<usize>,
  ) -> Vec<(String, Vec<Piece>)> {
    scan(&chars(lines), wraps)
      .into_iter()
      .map(|found| (found.target, found.pieces))
      .collect()
  }

  fn found(lines: &[&str]) -> Vec<(String, Vec<Piece>)> {
    found_with(lines, |line| (line + 1 < lines.len()).then_some(line + 1))
  }

  /// Lines, then each address and its pieces.
  type Case<'a> = (&'a [&'a str], &'a [(&'a str, &'a [Piece])]);

  #[test]
  fn addresses_found_in_text() {
    let cases: &[Case] = &[
      (&["see https://a.org/x"], &[("https://a.org/x", &[(0, 4..19)])]),
      (&["https://a.org."], &[("https://a.org", &[(0, 0..13)])]),
      (&["(https://a.org)."], &[("https://a.org", &[(0, 1..14)])]),
      (
        &["(see https://en.wikipedia.org/wiki/A_(b))"],
        &[("https://en.wikipedia.org/wiki/A_(b)", &[(0, 5..40)])],
      ),
      (&["[http://a.org/x]"], &[("http://a.org/x", &[(0, 1..15)])]),
      (&["\"https://a.org\","], &[("https://a.org", &[(0, 1..14)])]),
      (&["HTTPS://A.ORG"], &[("https://A.ORG", &[(0, 0..13)])]),
      (
        &["at www.irs.gov/Form1040."],
        &[("http://www.irs.gov/Form1040", &[(0, 3..23)])],
      ),
      (
        &["https://doi.org/10.5194/egusphere-2025-2374"],
        &[("https://doi.org/10.5194/egusphere-2025-2374", &[(0, 0..43)])],
      ),
      (
        &["https://a.jp/", "b/c next"],
        &[("https://a.jp/b/c", &[(0, 0..13), (1, 0..3)])],
      ),
      (
        &["at https://a-", "  b.org/x."],
        &[("https://a-b.org/x", &[(0, 3..13), (1, 2..9)])],
      ),
      (
        &["https://doi.org/10.5194/egusphere-", "2025-2374"],
        &[(
          "https://doi.org/10.5194/egusphere-2025-2374",
          &[(0, 0..34), (1, 0..9)],
        )],
      ),
      (&["https://a.org.", "Next"], &[("https://a.org", &[(0, 0..13)])]),
      (&["https://a.org/ next", "line"], &[("https://a.org/", &[(0, 0..14)])]),
      (
        &["two http://a.org and www.b.com"],
        &[
          ("http://a.org", &[(0, 4..16)]),
          ("http://www.b.com", &[(0, 21..30)]),
        ],
      ),
      (&["https://a.org/\u{fb01}le"], &[("https://a.org/file", &[(0, 0..17)])]),
      (&["https://a.org/x\u{b9}"], &[("https://a.org/x", &[(0, 0..15)])]),
      (
        &["\u{898b}\u{3066}https://a.org/x\u{3092}"],
        &[("https://a.org/x", &[(0, 2..17)])],
      ),
      (
        &["see www.example.com\u{c5d0}\u{c11c}"],
        &[("http://www.example.com", &[(0, 4..19)])],
      ),
      (
        &["https://a.org/", "https://b.org/x"],
        &[
          ("https://a.org/", &[(0, 0..14)]),
          ("https://b.org/x", &[(1, 0..15)]),
        ],
      ),
    ];
    check(cases);
  }

  #[test]
  fn dois_and_mail_found_in_text() {
    let cases: &[Case] = &[
      (&["mail mailto:me@a.org;"], &[("mailto:me@a.org", &[(0, 5..20)])]),
      (
        &["doi:10.1000/xyz123."],
        &[("https://doi.org/10.1000/xyz123", &[(0, 0..18)])],
      ),
      (
        &["DOI: 10.5194/acp-1-2"],
        &[("https://doi.org/10.5194/acp-1-2", &[(0, 5..20)])],
      ),
      (
        &["doi.org/10.5194/x and dx.doi.org/10.1000/y"],
        &[
          ("https://doi.org/10.5194/x", &[(0, 0..17)]),
          ("https://doi.org/10.1000/y", &[(0, 22..42)]),
        ],
      ),
      (
        &["doi.org/10.1000/a%23b"],
        &[("https://doi.org/10.1000/a%23b", &[(0, 0..21)])],
      ),
      (
        &["avaswani@google.com and (loewe@slf.ch)"],
        &[
          ("mailto:avaswani@google.com", &[(0, 0..19)]),
          ("mailto:loewe@slf.ch", &[(0, 25..37)]),
        ],
      ),
      (
        &["E-mail:jdoe@uni.edu or 'monty@python.org'."],
        &[
          ("mailto:jdoe@uni.edu", &[(0, 7..19)]),
          ("mailto:monty@python.org", &[(0, 24..40)]),
        ],
      ),
      (
        &["https://a.org/", "me@b.org"],
        &[("https://a.org/", &[(0, 0..14)]), ("mailto:me@b.org", &[(1, 0..8)])],
      ),
      (
        &["mail feedback@thinkpython.com."],
        &[("mailto:feedback@thinkpython.com", &[(0, 5..29)])],
      ),
      (
        &["a.b+c@x-y.uni-z.de,d@e.org"],
        &[
          ("mailto:a.b+c@x-y.uni-z.de", &[(0, 0..18)]),
          ("mailto:d@e.org", &[(0, 19..26)]),
        ],
      ),
      (
        &["doi:10.1000/a#b?c"],
        &[("https://doi.org/10.1000/a%23b%3Fc", &[(0, 0..17)])],
      ),
      (
        &["\u{2217}ab@x.org \u{201C}cd@y.org\u{201D} {ef@z.org}"],
        &[
          ("mailto:ab@x.org", &[(0, 1..9)]),
          ("mailto:cd@y.org", &[(0, 11..19)]),
          ("mailto:ef@z.org", &[(0, 22..30)]),
        ],
      ),
      (
        &["(gh@x.org.) \u{2014}ij@y.org\u{2014}"],
        &[
          ("mailto:gh@x.org", &[(0, 1..9)]),
          ("mailto:ij@y.org", &[(0, 13..21)]),
        ],
      ),
    ];
    check(cases);
  }

  #[test]
  fn wrapped_mail_is_not_guessed() {
    // The `url` package breaks addresses after `.`, `-`, and `@`.
    let cases: &[&[&str]] = &[
      &["write john.", "smith@cs.toronto.edu now"],
      &["mary-", "jane@uni.edu"],
      &["jdoe@mail.google.", "com"],
    ];
    for lines in cases {
      assert_eq!(found(lines), vec![], "{lines:?}");
    }
    // Without a wrap the row ends a sentence.
    let alone = found_with(&["mail jdoe@mail.google."], |_| None);
    assert_eq!(
      alone,
      [("mailto:jdoe@mail.google".to_owned(), vec![(0, 5..21)])]
    );
  }

  fn check(cases: &[Case]) {
    for (lines, want) in cases {
      let want: Vec<_> = want
        .iter()
        .map(|(target, pieces)| ((*target).to_owned(), pieces.to_vec()))
        .collect();
      assert_eq!(found(lines), want, "{lines:?}");
    }
  }

  #[test]
  fn plain_text_has_no_addresses() {
    let cases = [
      "e.g. this, i.e. that",
      "version 10.5 and 10.12.3",
      "pages 10.1-10.5",
      "10.123/x has too few digits",
      "10.1234/ has no suffix",
      "http:// alone",
      "https://.",
      "www. alone",
      "www.nodot",
      "mailto:nobody",
      "xhttp://a.org",
      "me@https://a.org",
      "a.www.b.org",
      "x10.1234/abc",
      "use a mailto: link",
      "mailto:.",
      "https: no",
      "https://a.org/x-",
      "https://a.org/x\u{2010}",
      "more@ and addr.split('@')",
      "@handle and a@b and a@b.c1",
      ".a@b.org and a.@b.org",
      "user@-x.org and user@x-.org",
      "x@y.c",
      "git@github.com:org/repo",
      "root@srv1.example.com:~$",
      "q@k.transpose(-2, -1)",
      "o'brien@x.org, user=foo@x.org, a!b@x.org",
      "%41@b.org",
      "ends@uni-heidelberg.deThe",
      "user:pass@host.example.org",
      "amqp://guest:guest@rabbit.example.org",
      "redis://:secret@cache.example.org",
      "sip:alice@example.org",
      "icon@2x.png and logo@3x.JPG",
      "a..b@x.org",
    ];
    for line in cases {
      assert_eq!(found(&[line]), vec![], "{line}");
    }
  }

  #[test]
  fn wraps_follow_the_named_line() {
    let lines = ["https://a.org/x-", "www.b.org", "y z", "https://doi.org/"];
    let lines = [&lines[..], &["10.5194/abc"]].concat();
    // Line 0 goes on in line 2, past line 1; line 3 in line 4.
    let wraps = |line| match line {
      0 => Some(2),
      3 => Some(4),
      _ => None,
    };
    let want = [
      ("https://a.org/x-y", vec![(0, 0..16), (2, 0..1)]),
      ("http://www.b.org", vec![(1, 0..9)]),
      ("https://doi.org/10.5194/abc", vec![(3, 0..16), (4, 0..11)]),
    ];
    let want: Vec<_> = want
      .into_iter()
      .map(|(target, pieces)| (target.to_owned(), pieces))
      .collect();
    assert_eq!(found_with(&lines, wraps), want);
    let unwrapped = found_with(&["https://a.jp/", "b/c"], |_| None);
    assert_eq!(unwrapped, [("https://a.jp/".to_owned(), vec![(0, 0..13)])]);
  }
}
