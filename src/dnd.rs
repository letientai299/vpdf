use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;
use vpdf_terminal::dnd::Dropped;

/// Pastes of more lines name no files.
const LINES: usize = 16;

/// What a drop gives the reader.
#[derive(Debug, Eq, PartialEq)]
pub enum Target {
  /// Text for an open prompt.
  Text(String),
  /// A PDF to open.
  Pdf(PathBuf),
}

/// File drops open PDFs; text edits prompts.
pub fn target(drop: &Dropped, typing: bool) -> crate::Result<Target> {
  if drop.files() {
    return first_pdf(drop.text())
      .map(Target::Pdf)
      .ok_or_else(|| anyhow::anyhow!("No PDF in the drop"));
  }
  if typing {
    return Ok(Target::Text(drop.text().to_owned()));
  }
  anyhow::bail!("No PDF in the drop")
}

/// The first existing PDF a paste names; long pastes name none.
pub fn pdf(text: &str) -> Option<PathBuf> {
  (text.lines().count() <= LINES).then(|| first_pdf(text)).flatten()
}

/// The first existing PDF a file list names: `file://` URIs, plain
/// paths, or shell words, one or more per line.
fn first_pdf(text: &str) -> Option<PathBuf> {
  entries(text)
    .flat_map(candidates)
    .find(|path| crate::cmdline::is_pdf(path) && path.is_file())
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
fn candidates(line: &str) -> Vec<PathBuf> {
  if let Some(rest) = line.strip_prefix("file://") {
    return local(rest).into_iter().collect();
  }
  let mut paths = vec![PathBuf::from(line)];
  paths.extend(words(line).into_iter().map(PathBuf::from));
  paths
}

/// The path of a `file://` URI on this machine; other hosts name none.
fn local(rest: &str) -> Option<PathBuf> {
  let start = rest.find('/')?;
  let host = &rest[..start];
  (host.is_empty() || host == "localhost").then(|| decoded(&rest[start..]))
}

/// Percent-decoded bytes, which need not be UTF-8.
fn decoded(text: &str) -> PathBuf {
  let mut bytes = Vec::with_capacity(text.len());
  bytes.extend(percent_encoding::percent_decode(text.as_bytes()));
  PathBuf::from(std::ffi::OsString::from_vec(bytes))
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
      (true, uri.clone(), true, Some(Target::Pdf(pdf.clone()))),
      (true, remote.clone(), true, None),
      (true, "# none\r\n".into(), true, None),
      (false, "two words".into(), true, text("two words")),
      (true, uri.clone(), false, Some(Target::Pdf(pdf.clone()))),
      // Drops name any number of files; only pastes stop at `LINES`.
      (true, remote.repeat(LINES) + &uri, false, { Some(Target::Pdf(pdf)) }),
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
  fn uris_decode_to_bytes() {
    // macOS refuses names that are not UTF-8, so no file backs these.
    let bytes =
      |bytes: &[u8]| PathBuf::from(std::ffi::OsString::from_vec(bytes.into()));
    let cases = [
      ("/a%20b.pdf", bytes(b"/a b.pdf")),
      ("/caf%E9.pdf", bytes(b"/caf\xe9.pdf")),
      ("/caf%e9.pdf", bytes(b"/caf\xe9.pdf")),
      ("/a+b%2b.pdf", bytes(b"/a+b+.pdf")),
      ("/%+5%zz%4", bytes(b"/%+5%zz%4")),
    ];
    for (text, want) in cases {
      assert_eq!(decoded(text), want, "{text}");
    }
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
      assert_eq!(pdf(&text), want, "{text:?}");
    }
    Ok(())
  }
}
