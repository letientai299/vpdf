use std::path::Path;
use vpdf_model::reference::Reference;
use vpdf_terminal::dnd::Dropped;

/// Pastes of more lines name no files.
const LINES: usize = 16;

/// What a drop gives the reader.
#[derive(Debug, PartialEq)]
pub enum Target {
  /// Text for an open prompt.
  Text(String),
  /// A PDF to open.
  Pdf(Reference),
}

/// File drops open PDFs; text edits prompts.
pub fn target(drop: &Dropped, typing: bool) -> crate::Result<Target> {
  if drop.files() {
    return first_pdf(drop.text())?
      .map(Target::Pdf)
      .ok_or_else(|| anyhow::anyhow!("No PDF in the drop"));
  }
  if typing {
    return Ok(Target::Text(drop.text().to_owned()));
  }
  anyhow::bail!("No PDF in the drop")
}

/// The first existing PDF a paste names; long pastes name none.
pub fn pdf(text: &str) -> crate::Result<Option<Reference>> {
  if text.lines().count() > LINES {
    return Ok(None);
  }
  first_pdf(text)
}

fn first_pdf(text: &str) -> crate::Result<Option<Reference>> {
  for line in entries(text) {
    let mut failure = None;
    for (index, input) in candidates(line).into_iter().enumerate() {
      let explicit = reference_input(&input);
      match crate::cmdline::reference(&input) {
        Ok(reference)
          if crate::cmdline::is_pdf(&reference.path)
            && reference.path.is_file() =>
        {
          return Ok(Some(reference));
        }
        Ok(reference)
          if crate::cmdline::is_pdf(&reference.path)
            && (!reference.operations.is_empty() || file_uri(&input)) =>
        {
          let error = anyhow::anyhow!("PDF file missing");
          if index > 0 {
            return Err(error);
          }
          failure = Some(error);
        }
        Err(error) if explicit && index > 0 => return Err(error),
        Err(error) if explicit => failure = Some(error),
        _ => {}
      }
    }
    if let Some(error) = failure {
      return Err(error);
    }
  }
  Ok(None)
}

fn file_uri(text: &str) -> bool {
  text.get(..5).is_some_and(|s| s.eq_ignore_ascii_case("file:"))
}

fn reference_input(text: &str) -> bool {
  text.contains("://")
    || file_uri(text)
    || text
      .split_once('#')
      .is_some_and(|(path, _)| crate::cmdline::is_pdf(Path::new(path)))
}

/// Lines but blanks and `#` comments, trimmed.
fn entries(text: &str) -> impl Iterator<Item = &str> {
  text
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty() && !line.starts_with('#'))
}

/// Paths a line may name: a local `file://` URI, the line as typed, or
/// its shell words.
fn candidates(line: &str) -> Vec<String> {
  let mut inputs = vec![line.to_owned()];
  if !file_uri(line) {
    inputs.extend(words(line));
  }
  inputs
}

/// Shell words: blanks split, backslashes escape, quotes group.
fn words(line: &str) -> Vec<String> {
  let mut words = Vec::new();
  let mut word = String::new();
  let mut started = false;
  let mut quote = None;
  let mut chars = line.chars();
  while let Some(ch) = chars.next() {
    match (quote, ch) {
      (Some(open), ch) if ch == open => quote = None,
      (Some('"') | None, '\\') => {
        word.extend(chars.next());
        started = true;
      }
      (Some(_), ch) => word.push(ch),
      (None, '\'' | '"') => {
        quote = Some(ch);
        started = true;
      }
      (None, ch) if ch.is_whitespace() => {
        if std::mem::take(&mut started) {
          words.push(std::mem::take(&mut word));
        }
      }
      (None, ch) => {
        word.push(ch);
        started = true;
      }
    }
  }
  if started {
    words.push(word);
  }
  words
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_terminal::dnd::{Drops, Step};
  #[test]
  fn drops_distinguish_payloads() -> std::io::Result<()> {
    let directory = tempfile::tempdir()?;
    let pdf = directory.path().join("a b.pdf");
    std::fs::write(&pdf, b"%PDF")?;
    let shown = pdf.display().to_string();
    let uri = format!("file://{}\r\n", shown.replace(' ', "%20"));
    let remote = "file://elsewhere/x.pdf\r\n".to_owned();
    let drop = |files, data: &str| {
      use base64::Engine;
      let mut drops = Drops::default();
      let mime = if files { "text/uri-list" } else { "text/plain" };
      drops.event(&format!("t=M:x=0:y=0:o=1;{mime}"), true);
      let encoded = base64::engine::general_purpose::STANDARD.encode(data);
      drops.event(&format!("t=r;{encoded}"), true);
      let Step::Dropped(drop) = drops.event("t=r", true) else {
        panic!("Drop missing");
      };
      drop
    };
    let text = |text: &str| Some(Target::Text(text.into()));
    // (payload, data, a prompt open) -> target
    let cases = [
      (true, uri.clone(), true, Some(Target::Pdf(pdf.clone().into()))),
      (true, remote.clone(), true, None),
      (true, "# none\r\n".into(), true, None),
      (false, "two words".into(), true, text("two words")),
      (true, uri.clone(), false, Some(Target::Pdf(pdf.clone().into()))),
      (true, remote.repeat(LINES) + &uri, false, None),
      // Only pastes stop at `LINES`.
      (
        true,
        "# comment\n".repeat(LINES) + &uri,
        false,
        Some(Target::Pdf(pdf.into())),
      ),
      (true, remote, false, None),
      (false, shown, false, None),
    ];
    for (payload, data, typing, want) in cases {
      let got = target(&drop(payload, &data), typing).ok();
      assert_eq!(got, want, "{data:?} typing={typing}");
    }
    Ok(())
  }

  #[test]
  fn lines_name_existing_pdfs() -> std::io::Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    for name in ["a b.pdf", "c.PDF", "notes.txt", "it's.pdf"] {
      std::fs::write(root.join(name), b"%PDF")?;
    }
    let at = |name: &str| root.join(name);
    let shown = |name: &str| at(name).display().to_string();
    let uri = format!("file://{}", shown("a b.pdf").replace(' ', "%20"));
    let host = format!("file://localhost{}", shown("c.PDF"));
    let remote = format!("file://elsewhere{}", shown("c.PDF"));
    let escaped = shown("a b.pdf").replace(' ', "\\ ");
    let quoted = format!("'{}'", shown("a b.pdf"));
    let posix = format!("'{}'", shown("it's.pdf").replace('\'', "'\\''"));
    let apostrophe = shown("it's.pdf").replace('\'', "\\'");
    let words = format!("{} {escaped}", shown("notes.txt"));
    let many = format!("{}\n", shown("c.PDF")).repeat(LINES + 1);
    // (text) -> the first PDF it names
    let cases = [
      (shown("a b.pdf"), Some(at("a b.pdf"))),
      (format!("  {}\n", shown("c.PDF")), Some(at("c.PDF"))),
      (uri.clone(), Some(at("a b.pdf"))),
      (host, Some(at("c.PDF"))),
      (remote, None),
      (escaped, Some(at("a b.pdf"))),
      (quoted, Some(at("a b.pdf"))),
      (posix, Some(at("it's.pdf"))),
      (apostrophe, Some(at("it's.pdf"))),
      (words, Some(at("a b.pdf"))),
      (format!("# comment\r\n{uri}\r\n{}", shown("c.PDF")), {
        Some(at("a b.pdf"))
      }),
      (shown("notes.txt"), None),
      (shown("missing.pdf"), None),
      ("hello world".into(), None),
      (many, None),
    ];
    for (text, want) in cases {
      assert_eq!(pdf(&text).ok().flatten().map(|r| r.path), want, "{text:?}");
    }
    Ok(())
  }
  #[test]
  fn explicit_failures_stop_file_lists() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("book.PDF");
    std::fs::write(&path, b"%PDF")?;
    for input in [
      format!("{}#page=0\n{}", path.display(), path.display()),
      format!("{}#page=0 {}", path.display(), path.display()),
      format!("file://elsewhere/missing.pdf\n{}", path.display()),
      format!("{}#page=2", root.path().join("missing.pdf").display()),
    ] {
      assert!(pdf(&input).is_err(), "{input}");
    }
    let reference = pdf(&format!("'{}#page=2'", path.display()))?.unwrap();
    assert_eq!(reference.path, path);
    assert_eq!(reference.operations.len(), 1);
    for fragment in ["nameddest=proof", "comment=note", "structelem=section"] {
      for prefix in ["", "file://"] {
        let reference =
          pdf(&format!("'{prefix}{}#{fragment}'", path.display()))?.unwrap();
        assert_eq!(reference.path, path);
        assert_eq!(reference.operations.len(), 1);
      }
    }
    let reference =
      pdf(&format!("notes.txt {}#page=2", path.display()))?.unwrap();
    assert_eq!(reference.path, path);
    assert_eq!(reference.operations.len(), 1);
    Ok(())
  }
}
