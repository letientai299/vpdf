//! Dropped files: the [Kitty drag and drop protocol][dnd], OSC 72, and
//! pasted paths where the terminal lacks it.
//!
//! [dnd]: https://sw.kovidgoyal.net/kitty/dnd-protocol/
use base64::Engine;
use base64::alphabet::STANDARD;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

/// Asks for drops of file lists.
pub const ACCEPT: &[u8] = b"\x1b]72;t=a;text/uri-list\x1b\\";
/// Ends drops, at exit and suspend.
pub const REFUSE: &[u8] = b"\x1b]72;t=A\x1b\\";

const URI_LIST: &str = "text/uri-list";

/// Dropped data past this size is cut.
const LIMIT: usize = 1 << 20;

/// MIME lists past this size are cut.
const MIMES: usize = 1 << 16;

/// Padding is optional at the end of the last chunk.
const BASE64: GeneralPurpose = GeneralPurpose::new(
  &STANDARD,
  GeneralPurposeConfig::new()
    .with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Ends a drop the reader did not take.
pub const CANCEL: &str = "\x1b]72;t=r:o=0\x1b\\";

/// What the reader does with dropped text, as the terminal allowed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Operation {
  Copy = 1,
  Move = 2,
}

/// What a drop carries and how the reader takes it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Payload {
  /// A `text/uri-list`, always copied: a source may delete moved files.
  Files,
  /// Plain text, taken only while a prompt is open.
  Text(Operation),
}

impl Payload {
  fn operation(self) -> Operation {
    match self {
      Self::Files => Operation::Copy,
      Self::Text(operation) => operation,
    }
  }
}

/// The drop being read.
#[derive(Debug)]
struct Reading {
  cell: (u16, u16),
  payload: Payload,
  data: Vec<u8>,
  /// Base64 past the last whole quantum, for the next chunk.
  carry: String,
}

/// The drag over the window and the drop being read.
#[derive(Debug, Default)]
pub struct Drops {
  /// A chunked event: its first metadata and its payload so far, up to
  /// `MIMES` bytes; dropped data decodes as it arrives instead.
  chunked: Option<(String, String)>,
  reading: Option<Reading>,
}

/// A finished drop.
#[derive(Debug, Eq, PartialEq)]
pub struct Dropped {
  /// The screen cell it landed on.
  pub cell: (u16, u16),
  payload: Payload,
  data: String,
}

impl Dropped {
  pub fn files(&self) -> bool {
    self.payload == Payload::Files
  }
  pub fn text(&self) -> &str {
    &self.data
  }

  /// The reply that ends the drop: what the reader did, or cancelled.
  pub fn finish(&self, done: bool) -> String {
    if done {
      osc(&format!("t=r:o={}", self.payload.operation() as u8))
    } else {
      CANCEL.into()
    }
  }
}

/// What one OSC 72 event asks of the reader.
#[derive(Debug, Eq, PartialEq)]
pub enum Step {
  Wait,
  /// Bytes for the terminal.
  Reply(String),
  Dropped(Dropped),
  /// The terminal ended the drop; the reader still sends `CANCEL`.
  Failed(String),
}

impl Drops {
  /// Acts on the body of an OSC 72 event; `typing` while a prompt takes
  /// text, which then accepts dragged text too.
  pub fn event(&mut self, body: &str, typing: bool) -> Step {
    let (own, payload) = body.split_once(';').unwrap_or((body, ""));
    let more = value(own, 'm') == Some("1");
    let (meta, mut text) = match self.chunked.take() {
      // Later chunks may carry only `m`; one naming another type is new.
      Some((first, text))
        if value(own, 't').is_none_or(|t| value(&first, 't') == Some(t)) =>
      {
        (first, text)
      }
      _ => (own.to_owned(), String::new()),
    };
    let kind = value(&meta, 't').and_then(|t| t.chars().next());
    if kind == Some('r') {
      self.read(payload);
    } else if text.len() < MIMES {
      text.push_str(payload);
    }
    if more {
      self.chunked = Some((meta, text));
      return Step::Wait;
    }
    match kind {
      Some('m') => self.moved(&meta, &text, typing),
      Some('M') => self.drop(&meta, &text, typing),
      Some('r') if payload.is_empty() => self.finished(),
      // The text is the error.
      Some('R') => match self.reading.take() {
        Some(_) => Step::Failed(text),
        None => Step::Wait,
      },
      _ => Step::Wait,
    }
  }

  /// A drag over the window: accept what the reader can use.
  fn moved(&mut self, meta: &str, mimes: &str, typing: bool) -> Step {
    if number(meta, 'x') == Some(-1) {
      // The drag left; anything still read is stale.
      self.reading = None;
      return Step::Wait;
    }
    if mimes.is_empty() {
      return Step::Wait;
    }
    Step::Reply(match taken(meta, mimes, typing) {
      Some((_, mime, payload)) => {
        osc(&format!("t=m:o={};{mime}", payload.operation() as u8))
      }
      None => osc("t=m:o=0"),
    })
  }

  /// The drop: ask for the data the reader wants.
  fn drop(&mut self, meta: &str, mimes: &str, typing: bool) -> Step {
    let cell = |key| number(meta, key).and_then(|v| u16::try_from(v).ok());
    let (Some((index, _, payload)), Some(x), Some(y)) =
      (taken(meta, mimes, typing), cell('x'), cell('y'))
    else {
      return Step::Reply(CANCEL.into());
    };
    let (data, carry) = (Vec::new(), String::new());
    self.reading = Some(Reading { cell: (x, y), payload, data, carry });
    Step::Reply(osc(&format!("t=r:x={}", index + 1)))
  }

  /// One chunk of dropped data; chunks may split a base64 quantum.
  fn read(&mut self, payload: &str) {
    let Some(reading) = &mut self.reading else { return };
    // Data cut at the limit still names its first files.
    if reading.data.len() >= LIMIT {
      return;
    }
    reading.carry.push_str(payload.trim());
    let whole = reading.carry.len() / 4 * 4;
    let rest = reading.carry.split_off(whole);
    let quanta = std::mem::replace(&mut reading.carry, rest);
    if let Ok(bytes) = BASE64.decode(quanta) {
      reading.data.extend_from_slice(&bytes);
    }
  }

  fn finished(&mut self) -> Step {
    let Some(mut reading) = self.reading.take() else { return Step::Wait };
    // The last chunk may leave its quantum unpadded.
    if let Ok(bytes) = BASE64.decode(&reading.carry) {
      reading.data.extend_from_slice(&bytes);
    }
    Step::Dropped(Dropped {
      cell: reading.cell,
      payload: reading.payload,
      data: String::from_utf8_lossy(&reading.data).into_owned(),
    })
  }
}

/// The offered type the reader takes, its index, and how: files to
/// copy; else, while a prompt is open, text to copy or move.
fn taken<'a>(
  meta: &str,
  mimes: &'a str,
  typing: bool,
) -> Option<(usize, &'a str, Payload)> {
  let allowed = number(meta, 'o').unwrap_or(0);
  let copy = allowed & 1 != 0;
  let offered = || mimes.split_whitespace().enumerate();
  let files = offered().find(|(_, mime)| *mime == URI_LIST);
  if let Some((index, mime)) = files.filter(|_| copy) {
    return Some((index, mime, Payload::Files));
  }
  let operation = match (copy, allowed & 2 != 0) {
    (true, _) => Operation::Copy,
    (false, true) => Operation::Move,
    (false, false) => return None,
  };
  let text = offered().find(|(_, mime)| mime.starts_with("text/plain"));
  let (index, mime) = text.filter(|_| typing)?;
  Some((index, mime, Payload::Text(operation)))
}

fn osc(body: &str) -> String {
  format!("\x1b]72;{body}\x1b\\")
}

/// The value of `key` in `k=v:k=v` metadata.
fn value(meta: &str, key: char) -> Option<&str> {
  meta.split(':').find_map(|pair| {
    let (name, value) = pair.split_once('=')?;
    (name.len() == 1 && name.starts_with(key)).then_some(value)
  })
}

fn number(meta: &str, key: char) -> Option<i64> {
  value(meta, key).and_then(|value| value.parse().ok())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn reply(body: &str) -> Step {
    Step::Reply(osc(body))
  }

  #[test]
  fn drops_follow_the_protocol() {
    let list = BASE64.encode("file:///tmp/a%20b.pdf\r\n");
    // Chunks may split a base64 quantum.
    let (first, rest) = list.split_at(5);
    let (second, third) = rest.split_at(6);
    let files = Dropped {
      cell: (7, 2),
      payload: Payload::Files,
      data: "file:///tmp/a%20b.pdf\r\n".into(),
    };
    let text = |operation| Dropped {
      cell: (1, 1),
      payload: Payload::Text(operation),
      data: "two words".into(),
    };
    // Kitty leaves the last quantum unpadded.
    let words =
      base64::engine::general_purpose::STANDARD_NO_PAD.encode("two words!");
    let text =
      |operation| Dropped { data: "two words!".into(), ..text(operation) };
    // (event body, a prompt open) -> step, in order, on one session
    let cases = [
      ("t=m:x=3:y=4:X=30:Y=40:o=3:m=0;text/plain text/uri-list ", false, {
        reply("t=m:o=1;text/uri-list")
      }),
      ("t=m:x=4:y=4:X=38:Y=40:o=3", false, Step::Wait),
      ("t=m:x=-1:y=-1", false, Step::Wait),
      ("t=m:x=3:y=4:o=3:m=0;text/plain", false, reply("t=m:o=0")),
      ("t=m:x=3:y=4:o=3:m=0;text/plain", true, reply("t=m:o=1;text/plain")),
      // Files never move: a source may delete what moved.
      ("t=m:x=3:y=4:o=2:m=0;text/uri-list", false, reply("t=m:o=0")),
      ("t=m:x=3:y=4:o=2:m=0;text/plain", true, reply("t=m:o=2;text/plain")),
      ("t=m:x=3:y=4:o=0:m=0;text/uri-list", false, reply("t=m:o=0")),
      ("t=M:x=1:y=1:o=2;text/uri-list", false, Step::Reply(CANCEL.into())),
      // A chunked MIME list keeps its first metadata.
      ("t=M:x=7:y=2:X=70:Y=20:o=3:m=1;text/plain te", false, Step::Wait),
      ("m=0;xt/uri-list", false, reply("t=r:x=2")),
      (&format!("t=r:x=2:m=1;{first}"), false, Step::Wait),
      (&format!("m=1;{second}"), false, Step::Wait),
      (&format!("m=0;{third}"), false, Step::Wait),
      ("t=r:x=2", false, Step::Dropped(files)),
      ("t=R:x=2;EPERM:late", false, Step::Wait),
      ("t=M:x=1:y=1:o=3;text/plain", false, Step::Reply(CANCEL.into())),
      ("t=M:x=1:y=1:o=3;text/uri-list", false, reply("t=r:x=1")),
      ("t=R:x=1;EIO:read failed", false, {
        Step::Failed("EIO:read failed".into())
      }),
      ("t=r:x=1", false, Step::Wait),
      // A stale chunk gives way to an event of another type.
      ("t=M:x=1:y=1:o=3:m=1;text/", false, Step::Wait),
      ("t=m:x=-1:y=-1", false, Step::Wait),
      ("t=M:x=1:y=1:o=3;text/plain", true, reply("t=r:x=1")),
      (&format!("t=r:x=1;{words}"), true, Step::Wait),
      ("t=r:x=1", true, Step::Dropped(text(Operation::Copy))),
      ("t=M:x=1:y=1:o=2;text/plain", true, reply("t=r:x=1")),
      (&format!("t=r:x=1;{words}"), true, Step::Wait),
      ("t=r:x=1", true, Step::Dropped(text(Operation::Move))),
      ("t=q;", false, Step::Wait),
    ];
    let mut drops = Drops::default();
    for (body, typing, step) in cases {
      assert_eq!(drops.event(body, typing), step, "{body}");
    }
    assert_eq!(text(Operation::Move).finish(true), osc("t=r:o=2"));
    assert_eq!(text(Operation::Move).finish(false), CANCEL);
  }

  #[test]
  fn data_past_the_limit_is_cut() {
    let mut drops = Drops::default();
    drops.event("t=M:x=1:y=1:o=1;text/uri-list", false);
    let chunk = BASE64.encode([b'a'; 3072]);
    for _ in 0..LIMIT / 3072 + 4 {
      drops.event(&format!("t=r:x=1:m=1;{chunk}"), false);
    }
    let Step::Dropped(drop) = drops.event("t=r:x=1", false) else {
      panic!("a drop");
    };
    assert!(drop.data.len() < LIMIT + 3072);
  }
}
