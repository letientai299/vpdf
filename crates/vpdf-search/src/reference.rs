use crate::text::Position;
use fancy_regex::Regex;
use std::sync::OnceLock;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Lookup {
  pub reference: Reference,
  pub origin: Position,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reference {
  kind: Kind,
  label: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
  Section,
  Chapter,
  Appendix,
  Figure,
  Table,
  Equation,
}

fn references() -> &'static Regex {
  static PATTERN: OnceLock<Regex> = OnceLock::new();
  PATTERN.get_or_init(|| {
    Regex::new(concat!(
      r"(?i)\b(section|sec\.|chapter|appendix|figure|fig\.|table|",
      r"equation|eq\.)\s+\(?",
      r"((?:[a-z]\.?)?[0-9]+(?:\.[0-9]+)*[a-z]?|[a-z])",
      r"(?!\w|\.[0-9])\)?"
    ))
    .expect("Reference pattern")
  })
}

fn kind(name: &str) -> Kind {
  match name.to_ascii_lowercase().as_str() {
    "chapter" => Kind::Chapter,
    "appendix" => Kind::Appendix,
    "figure" | "fig." => Kind::Figure,
    "table" => Kind::Table,
    "equation" | "eq." => Kind::Equation,
    _ => Kind::Section,
  }
}

impl Reference {
  pub fn at(line: &str, byte: usize) -> Option<Self> {
    for found in references().captures_iter(line) {
      let found = found.ok()?;
      let whole = found.get(0)?;
      if (whole.start()..whole.end()).contains(&byte) {
        return Some(Self {
          kind: kind(found.get(1)?.as_str()),
          label: found.get(2)?.as_str().to_ascii_lowercase(),
        });
      }
    }
    None
  }

  pub fn title(&self) -> String {
    let name = match self.kind {
      Kind::Section => "section",
      Kind::Chapter => "chapter",
      Kind::Appendix => "appendix",
      Kind::Figure => "figure",
      Kind::Table => "table",
      Kind::Equation => "equation",
    };
    format!("{name} {}", self.label)
  }

  pub fn heading(&self, title: &str) -> bool {
    if !matches!(self.kind, Kind::Section | Kind::Chapter | Kind::Appendix) {
      return false;
    }
    let title = title.trim_start();
    if let Some(reference) = Self::at(title, 0) {
      return reference == *self;
    }
    title.split_whitespace().next().is_some_and(|label| {
      label.trim_end_matches(['.', ':']).eq_ignore_ascii_case(&self.label)
    })
  }

  pub fn destination(&self, line: &str) -> Option<(usize, usize)> {
    let trimmed = line.trim_start();
    let start = line.len() - trimmed.len();
    if let Some(found) = references().captures(trimmed).ok().flatten()
      && let Some(whole) = found.get(0)
      && whole.start() == 0
      && Self::at(trimmed, 0).as_ref() == Some(self)
      && (trimmed[whole.end()..].trim().is_empty()
        || trimmed[whole.end()..].trim_start().starts_with([':', '.']))
    {
      return Some((start, start + whole.end()));
    }
    if self.label.chars().any(|ch| ch.is_ascii_digit()) && self.heading(trimmed)
    {
      let label = trimmed.split_whitespace().next()?;
      if label.trim_end_matches('.').eq_ignore_ascii_case(&self.label)
        && trimmed[label.len()..].chars().any(char::is_alphabetic)
      {
        return Some((start, start + label.len()));
      }
    }
    if self.kind == Kind::Equation {
      let end = line.trim_end().len();
      let marker = format!("({})", self.label);
      if line[..end].to_ascii_lowercase().ends_with(&marker)
        && (trimmed.trim_end().eq_ignore_ascii_case(&marker)
          || (line[..end - marker.len()].contains('=')
            && !references().is_match(line).unwrap_or(true)))
      {
        return Some((end - marker.len(), end));
      }
    }
    None
  }
}
