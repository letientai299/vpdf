//! Local document references and ordered PDF targets.
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::num::NonZeroUsize;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
  pub path: PathBuf,
  pub operations: Vec<Operation>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
  Page(NonZeroUsize),
  NamedDestination(Vec<u8>),
  Structure(Vec<u8>),
  Comment(Vec<u8>),
  Highlight([f32; 4]),
  Search(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(&'static str);

impl fmt::Display for Error {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(self.0)
  }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

impl From<PathBuf> for Reference {
  fn from(path: PathBuf) -> Self {
    Self { path, operations: Vec::new() }
  }
}

impl Reference {
  pub fn parse(input: &OsStr) -> Result<Self> {
    let bytes = input.as_bytes();
    let (path, fragment) = match bytes.iter().position(|&b| b == b'#') {
      Some(at) => (&bytes[..at], Some(&bytes[at + 1..])),
      None => (bytes, None),
    };
    let path =
      if path.get(..5).is_some_and(|s| s.eq_ignore_ascii_case(b"file:")) {
        uri_path(&path[5..])?
      } else {
        if scheme(path) {
          return Err(Error("Unsupported reference scheme"));
        }
        if path.contains(&0) {
          return Err(Error("NUL in document path"));
        }
        path.to_vec()
      };
    if path.is_empty() {
      return Err(Error("Empty document path"));
    }
    let operations = fragment.map_or_else(
      || Ok(Vec::new()),
      |fragment| {
        fragment.split(|b| matches!(b, b'&' | b'#')).map(operation).collect()
      },
    )?;
    Ok(Self { path: PathBuf::from(OsString::from_vec(path)), operations })
  }

  pub fn resolve(&self, base: &Path) -> PathBuf {
    base.join(&self.path)
  }
}

fn scheme(path: &[u8]) -> bool {
  let Some(at) = path.iter().position(|&b| b == b':') else { return false };
  at > 0
    && path[0].is_ascii_alphabetic()
    && path[..at]
      .iter()
      .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(b))
}

fn uri_path(mut path: &[u8]) -> Result<Vec<u8>> {
  if let Some(rest) = path.strip_prefix(b"//") {
    let at = rest.iter().position(|&b| b == b'/').unwrap_or(rest.len());
    let host = &rest[..at];
    if !host.is_empty() && !host.eq_ignore_ascii_case(b"localhost") {
      return Err(Error("Nonlocal file authority"));
    }
    path = &rest[at..];
  }
  if !path.starts_with(b"/") {
    return Err(Error("File URI requires absolute path"));
  }
  if path.contains(&b'?') {
    return Err(Error("File URI query unsupported"));
  }
  if !path
    .iter()
    .all(|b| b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/%".contains(b))
  {
    return Err(Error("Invalid file URI path"));
  }
  decode(path)
}

fn decode(input: &[u8]) -> Result<Vec<u8>> {
  let mut output = Vec::with_capacity(input.len());
  let mut index = 0;
  while index < input.len() {
    let byte = if input[index] == b'%' {
      let pair = input
        .get(index + 1..index + 3)
        .ok_or(Error("Malformed percent escape"))?;
      let digit = |b: u8| {
        (b as char).to_digit(16).ok_or(Error("Malformed percent escape"))
      };
      index += 2;
      (digit(pair[0])? * 16 + digit(pair[1])?) as u8
    } else {
      input[index]
    };
    if byte == 0 {
      return Err(Error("NUL in document reference"));
    }
    output.push(byte);
    index += 1;
  }
  Ok(output)
}

fn operation(input: &[u8]) -> Result<Operation> {
  let at = input
    .iter()
    .position(|&b| b == b'=')
    .ok_or(Error("Malformed PDF target"))?;
  let name = decode(&input[..at])?;
  let raw = &input[at + 1..];
  let value = decode(raw)?;
  if value.is_empty() {
    return Err(Error("Empty PDF target"));
  }
  match name.as_slice() {
    b"page" => {
      if !value.iter().all(u8::is_ascii_digit) {
        return Err(Error("Invalid physical page"));
      }
      let page = std::str::from_utf8(&value)
        .ok()
        .and_then(|s| s.parse().ok())
        .and_then(NonZeroUsize::new)
        .ok_or(Error("Invalid physical page"))?;
      Ok(Operation::Page(page))
    }
    b"nameddest" => Ok(Operation::NamedDestination(value)),
    b"structelem" => Ok(Operation::Structure(value)),
    b"comment" => Ok(Operation::Comment(value)),
    b"highlight" => {
      let values: Vec<f32> = raw
        .split(|&b| b == b',')
        .map(|part| {
          let bytes = decode(part)?;
          std::str::from_utf8(&bytes)
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .ok_or(Error("Invalid highlight rectangle"))
        })
        .collect::<Result<_>>()?;
      let rect: [f32; 4] =
        values.try_into().map_err(|_| Error("Invalid highlight rectangle"))?;
      if rect[0] < 0.0
        || rect[2] < 0.0
        || rect[0] >= rect[1]
        || rect[2] >= rect[3]
      {
        return Err(Error("Invalid highlight rectangle"));
      }
      Ok(Operation::Highlight(rect))
    }
    b"search" => {
      let text = std::str::from_utf8(&value)
        .map_err(|_| Error("Invalid search text"))?;
      let text = text
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or(Error("Search requires quoted words"))?;
      if text.contains('"') {
        return Err(Error("Invalid search text"));
      }
      let words: Vec<_> =
        text.split(' ').filter(|s| !s.is_empty()).map(str::to_owned).collect();
      if words.is_empty() {
        return Err(Error("Empty search words"));
      }
      Ok(Operation::Search(words))
    }
    _ => Err(Error("Unsupported PDF target parameter")),
  }
}

#[cfg(test)]
mod tests;
