//! Decodes terminal input bytes into keys, mouse reports, pastes, focus
//! changes, resizes, and query replies.
//!
//! crossterm 0.29 rejects SGR buttons above 3 and drops every reply, so
//! vpdf parses the stream itself. Keys stay crossterm `KeyEvent`s and
//! decode as crossterm does, except that Kitty's alternate keys and the
//! distinct Ctrl-i of Kitty and of tmux's extended keys survive.
//! Sources: [xterm control sequences][xterm], [Kitty keyboard][kbd],
//! [tmux extended keys][tmux].
//!
//! [xterm]: https://invisible-island.net/xterm/ctlseqs/ctlseqs.html
//! [kbd]: https://sw.kovidgoyal.net/kitty/keyboard-protocol/
//! [tmux]: https://github.com/tmux/tmux/blob/3.8/input-keys.c#L425-L473
use crossterm::event::{
  KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
};
use std::collections::VecDeque;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
  Key(KeyEvent),
  Mouse(Mouse),
  /// Kitty's pointer left the window; sent only under mode 1016.
  Leave,
  Paste(String),
  /// Mode 1004; true when the terminal gains focus.
  Focus(bool),
  /// SIGWINCH carries no size; a mode 2048 report does.
  Resize(Option<Size>),
  Reply(Reply),
}

/// A mode 2048 in-band resize report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Size {
  pub rows: u16,
  pub columns: u16,
  pub height: u16,
  pub width: u16,
}

/// Answers to terminal queries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Reply {
  /// DA1, `CSI ? 62 ; 22 c`.
  Attributes(Vec<u32>),
  /// DECRPM, `CSI ? mode ; state $ y`.
  Mode { mode: u32, state: ModeState },
  /// XTVERSION, `DCS > | name ST`.
  Version(String),
  /// `CSI … t` window reports, such as `6 ; height ; width` for the cell.
  Window(Vec<u32>),
  /// Kitty keyboard flags, `CSI ? flags u`.
  Keyboard(u32),
  /// Kitty graphics, `APC G i=id,p=placement ; message ST`; the message
  /// is `OK` or an error such as `ENOENT:…`.
  Graphics { id: Option<u32>, placement: Option<u32>, message: String },
  /// OSC 22, 52, 72, 5522, and any other OSC: its number and the rest.
  Osc { code: u32, body: String },
}

/// DECRPM states 0-4.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModeState {
  NotRecognized,
  Set,
  Reset,
  AlwaysSet,
  AlwaysReset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mouse {
  pub kind: MouseKind,
  pub at: Spot,
  pub modifiers: KeyModifiers,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseKind {
  Press(Button),
  Release(Button),
  Drag(Button),
  Moved,
}

/// Mouse button 1-11: left, middle, right, wheel up, down, left, right,
/// back, forward, and two more.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Button(u8);

impl Button {
  pub fn number(self) -> u8 {
    self.0
  }

  pub const LEFT: Self = Self(1);
  pub const BACK: Self = Self(8);
  pub const FORWARD: Self = Self(9);
}

/// Zero-based cell, or pixels as reported under mode 1016.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Spot {
  Cell { column: u16, row: u16 },
  Pixel { x: u16, y: u16 },
}

/// Caps an unterminated sequence or a paste.
const LIMIT: usize = 8 << 20;

const PASTE_END: &[u8] = b"\x1b[201~";

/// Keeps partial sequences between reads.
#[derive(Default)]
pub struct Parser {
  pending: Vec<u8>,
  /// Text of an open bracketed paste.
  paste: Option<Vec<u8>>,
  /// How far the partial sequence at the start of `pending` was searched.
  scanned: usize,
  /// The partial sequence passed `LIMIT`; its body was dropped, and it
  /// is skipped once it ends.
  overflow: bool,
  /// Mode 1016: SGR mouse reports carry pixels.
  pixels: bool,
}

enum Step {
  Emit(Event, usize),
  Skip(usize),
  Paste(usize),
  Partial,
}

impl Parser {
  /// Decodes `bytes`; a sequence cut short waits for the next call.
  pub fn feed(&mut self, bytes: &[u8], events: &mut VecDeque<Event>) {
    self.pending.extend_from_slice(bytes);
    let mut at = 0;
    while at < self.pending.len() {
      if let Some(paste) = &mut self.paste {
        let (used, done) = pasted(&self.pending[at..], paste);
        at += used;
        if !done {
          break;
        }
        let text = String::from_utf8_lossy(paste).into_owned();
        self.paste = None;
        events.push_back(Event::Paste(text));
        continue;
      }
      let resume = if at == 0 { self.scanned } else { 0 };
      let mut next = step(&self.pending[at..], self.pixels, resume);
      if at == 0 && self.overflow {
        next = match next {
          Step::Emit(_, used) | Step::Paste(used) => Step::Skip(used),
          other => other,
        };
        self.overflow = matches!(next, Step::Partial);
      }
      match next {
        Step::Emit(event, used) => {
          events.push_back(event);
          at += used;
        }
        Step::Skip(used) => at += used,
        Step::Paste(used) => {
          self.paste = Some(Vec::new());
          at += used;
        }
        Step::Partial => break,
      }
    }
    self.pending.drain(..at);
    if self.pending.len() > LIMIT {
      // Keep the introducer and a trailing ESC that may start ST.
      let last = self.pending.pop().filter(|&byte| byte == 0x1b);
      self.pending.truncate(2);
      self.pending.extend(last);
      self.overflow = true;
    }
    self.scanned = self.pending.len();
  }

  /// Mode 1016: reports carry pixels from now on.
  pub fn set_pixels(&mut self, pixels: bool) {
    self.pixels = pixels;
  }

  /// Whether a lone ESC waits for the rest of its sequence.
  pub fn waiting(&self) -> bool {
    self.paste.is_none() && self.pending == [0x1b]
  }

  /// Input went quiet: a lone ESC left over is the Esc key.
  pub fn end(&mut self, events: &mut VecDeque<Event>) {
    if self.waiting() {
      self.pending.clear();
      self.scanned = 0;
      events.push_back(key(KeyCode::Esc, KeyModifiers::NONE));
    }
  }
}

/// Moves paste text from `bytes`; true once the end marker passed. A
/// marker cut short stays unread for the next call.
fn pasted(bytes: &[u8], paste: &mut Vec<u8>) -> (usize, bool) {
  if let Some(end) = find(bytes, PASTE_END) {
    keep(paste, &bytes[..end]);
    return (end + PASTE_END.len(), true);
  }
  let tail = (1..PASTE_END.len())
    .rev()
    .find(|&size| bytes.ends_with(&PASTE_END[..size]))
    .unwrap_or(0);
  let used = bytes.len() - tail;
  keep(paste, &bytes[..used]);
  (used, false)
}

fn keep(paste: &mut Vec<u8>, bytes: &[u8]) {
  let room = LIMIT.saturating_sub(paste.len());
  paste.extend_from_slice(&bytes[..bytes.len().min(room)]);
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
  haystack.windows(needle.len()).position(|window| window == needle)
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
  Event::Key(KeyEvent::new(code, modifiers))
}

/// One event from the start of `bytes`; `resume` skips the part of an
/// unterminated string already searched.
fn step(bytes: &[u8], pixels: bool, resume: usize) -> Step {
  if bytes[0] != 0x1b {
    return plain(bytes);
  }
  let Some(&next) = bytes.get(1) else { return Step::Partial };
  match next {
    b'[' => csi(bytes, pixels, resume),
    b'O' => ss3(bytes),
    b']' | b'P' | b'_' | b'X' | b'^' => string(bytes, resume),
    0x1b => Step::Emit(key(KeyCode::Esc, KeyModifiers::NONE), 1),
    _ => match plain(&bytes[1..]) {
      Step::Emit(Event::Key(mut alt), used) => {
        alt.modifiers |= KeyModifiers::ALT;
        Step::Emit(Event::Key(alt), used + 1)
      }
      Step::Partial => Step::Partial,
      _ => Step::Skip(1),
    },
  }
}

/// A control byte or a UTF-8 character.
fn plain(bytes: &[u8]) -> Step {
  let control = |code, used| Step::Emit(key(code, KeyModifiers::CONTROL), used);
  match bytes[0] {
    b'\r' => Step::Emit(key(KeyCode::Enter, KeyModifiers::NONE), 1),
    b'\t' => Step::Emit(key(KeyCode::Tab, KeyModifiers::NONE), 1),
    0x7f => Step::Emit(key(KeyCode::Backspace, KeyModifiers::NONE), 1),
    0 => control(KeyCode::Char(' '), 1),
    byte @ 1..=0x1a => control(KeyCode::Char(char::from(byte - 1 + b'a')), 1),
    byte @ 0x1c..=0x1f => {
      control(KeyCode::Char(char::from(byte - 0x1c + b'4')), 1)
    }
    _ => utf8(bytes),
  }
}

fn utf8(bytes: &[u8]) -> Step {
  let size = match bytes[0] {
    0xc0..=0xdf => 2,
    0xe0..=0xef => 3,
    0xf0..=0xf7 => 4,
    0x80..=0xbf | 0xf8..=0xff => return Step::Skip(1),
    _ => 1,
  };
  let Some(encoded) = bytes.get(..size) else {
    let continued = bytes[1..].iter().all(|byte| byte & 0xc0 == 0x80);
    return if continued { Step::Partial } else { Step::Skip(1) };
  };
  let Some(ch) =
    std::str::from_utf8(encoded).ok().and_then(|s| s.chars().next())
  else {
    return Step::Skip(1);
  };
  let modifiers =
    if ch.is_uppercase() { KeyModifiers::SHIFT } else { KeyModifiers::NONE };
  Step::Emit(key(KeyCode::Char(ch), modifiers), size)
}

fn ss3(bytes: &[u8]) -> Step {
  let Some(&last) = bytes.get(2) else { return Step::Partial };
  match letter(last) {
    Some(code) => Step::Emit(key(code, KeyModifiers::NONE), 3),
    None => Step::Skip(3),
  }
}

fn letter(last: u8) -> Option<KeyCode> {
  Some(match last {
    b'A' => KeyCode::Up,
    b'B' => KeyCode::Down,
    b'C' => KeyCode::Right,
    b'D' => KeyCode::Left,
    b'E' => KeyCode::KeypadBegin,
    b'F' => KeyCode::End,
    b'H' => KeyCode::Home,
    b'P'..=b'S' => KeyCode::F(last - b'P' + 1),
    _ => return None,
  })
}

fn csi(bytes: &[u8], pixels: bool, resume: usize) -> Step {
  let mut end = resume.max(2);
  loop {
    let Some(&byte) = bytes.get(end) else { return Step::Partial };
    match byte {
      0x40..=0x7e => break,
      0x20..=0x3f => end += 1,
      // A control byte cuts the sequence short.
      _ => return Step::Skip(end),
    }
  }
  let body = &bytes[2..end];
  let last = bytes[end];
  let used = end + 1;
  let (marker, rest) = match body.first() {
    Some(&marker @ b'<'..=b'?') => (Some(marker), &body[1..]),
    _ => (None, body),
  };
  let split = rest.iter().position(|byte| (0x20..=0x2f).contains(byte));
  let (params, middle) = rest.split_at(split.unwrap_or(rest.len()));
  let event = match (marker, middle, last) {
    // `-` in pixel reports is an intermediate byte.
    (Some(b'<'), _, b'M' | b'm') => mouse(rest, last == b'm', pixels),
    (Some(b'?'), b"", b'c') => {
      // kitty ends the list with `;`.
      let fields = params.strip_suffix(b";").unwrap_or(params);
      Some(Event::Reply(Reply::Attributes(numbers(fields))))
    }
    (Some(b'?'), b"", b'u') => {
      Some(Event::Reply(Reply::Keyboard(number(params, 0, 0).unwrap_or(0))))
    }
    (Some(b'?'), b"$", b'y') => mode(params),
    (None, b"", b'I' | b'O') if params.is_empty() => {
      Some(Event::Focus(last == b'I'))
    }
    (None, b"", b't') => Some(window(params)),
    (None, b"", b'u') => kitty(params),
    (None, b"", b'~') if params == b"200" => return Step::Paste(used),
    (None, b"", b'~') if number(params, 0, 0) == Some(27) => other_keys(params),
    (None, b"", b'~') => tilde(params),
    (None, b"", b'Z') => Some(key(KeyCode::BackTab, KeyModifiers::SHIFT)),
    (None, b"", _) => letter(last).map(|code| modified(code, params)),
    _ => None,
  };
  match event {
    Some(event) => Step::Emit(event, used),
    None => Step::Skip(used),
  }
}

fn mode(params: &[u8]) -> Option<Event> {
  let mode = number(params, 0, 0)?;
  let state = match number(params, 1, 0)? {
    1 => ModeState::Set,
    2 => ModeState::Reset,
    3 => ModeState::AlwaysSet,
    4 => ModeState::AlwaysReset,
    _ => ModeState::NotRecognized,
  };
  Some(Event::Reply(Reply::Mode { mode, state }))
}

/// Field `index` of `;`-separated params, sub-field `sub` of its
/// `:`-separated parts; None when absent or empty.
fn number(params: &[u8], index: usize, sub: usize) -> Option<u32> {
  let field = params.split(|&byte| byte == b';').nth(index)?;
  let part = field.split(|&byte| byte == b':').nth(sub)?;
  if part.is_empty() || !part.iter().all(u8::is_ascii_digit) {
    return None;
  }
  Some(part.iter().fold(0_u32, |value, digit| {
    value.saturating_mul(10).saturating_add(u32::from(digit - b'0'))
  }))
}

fn numbers(params: &[u8]) -> Vec<u32> {
  let count = params.split(|&byte| byte == b';').count();
  (0..count).map(|index| number(params, index, 0).unwrap_or(0)).collect()
}

fn small(value: u32) -> u16 {
  u16::try_from(value).unwrap_or(u16::MAX)
}

/// Mode 2048 resize reports go out as resizes, the rest as replies.
fn window(params: &[u8]) -> Event {
  let values = numbers(params);
  if let [48, rows, columns, height, width] = values[..] {
    return Event::Resize(Some(Size {
      rows: small(rows),
      columns: small(columns),
      height: small(height),
      width: small(width),
    }));
  }
  Event::Reply(Reply::Window(values))
}

/// Kitty modifiers are one plus a bit set; lock bits become key state.
fn modifiers(mask: u32) -> (KeyModifiers, KeyEventState) {
  let bits = mask.saturating_sub(1);
  let modifiers = held(
    bits,
    &[
      (1, KeyModifiers::SHIFT),
      (2, KeyModifiers::ALT),
      (4, KeyModifiers::CONTROL),
      (8, KeyModifiers::SUPER),
      (16, KeyModifiers::HYPER),
      (32, KeyModifiers::META),
    ],
  );
  let mut state = KeyEventState::NONE;
  if bits & 64 != 0 {
    state |= KeyEventState::CAPS_LOCK;
  }
  if bits & 128 != 0 {
    state |= KeyEventState::NUM_LOCK;
  }
  (modifiers, state)
}

/// The modifiers whose bits are set in `bits`.
fn held(bits: u32, table: &[(u32, KeyModifiers)]) -> KeyModifiers {
  table
    .iter()
    .filter(|(bit, _)| bits & bit != 0)
    .fold(KeyModifiers::NONE, |held, (_, modifier)| held | *modifier)
}

fn kind(code: u32) -> KeyEventKind {
  match code {
    2 => KeyEventKind::Repeat,
    3 => KeyEventKind::Release,
    _ => KeyEventKind::Press,
  }
}

/// A legacy functional key; field 2 holds modifiers and the event type.
fn modified(code: KeyCode, params: &[u8]) -> Event {
  let (modifiers, state) = modifiers(number(params, 1, 0).unwrap_or(1));
  let kind = kind(number(params, 1, 1).unwrap_or(1));
  Event::Key(KeyEvent::new_with_kind_and_state(code, modifiers, kind, state))
}

fn tilde(params: &[u8]) -> Option<Event> {
  let code = match number(params, 0, 0)? {
    1 | 7 => KeyCode::Home,
    2 => KeyCode::Insert,
    3 => KeyCode::Delete,
    4 | 8 => KeyCode::End,
    5 => KeyCode::PageUp,
    6 => KeyCode::PageDown,
    value @ 11..=15 => KeyCode::F(value as u8 - 10),
    value @ 17..=21 => KeyCode::F(value as u8 - 11),
    value @ 23..=26 => KeyCode::F(value as u8 - 12),
    value @ 28..=29 => KeyCode::F(value as u8 - 15),
    value @ 31..=34 => KeyCode::F(value as u8 - 17),
    57427 => KeyCode::KeypadBegin,
    _ => return None,
  };
  Some(modified(code, params))
}

/// xterm's modifyOtherKeys form, `CSI 27 ; modifiers ; code ~`, which
/// tmux sends with `extended-keys-format xterm`; read as `CSI code ;
/// modifiers u`.
fn other_keys(params: &[u8]) -> Option<Event> {
  let mut fields = params.split(|&byte| byte == b';').skip(1);
  let (modifiers, code) = (fields.next()?, fields.next()?);
  kitty(&[code, b";", modifiers].concat())
}

/// `CSI code:shifted:base ; modifiers:type ; text u`. With alternate
/// keys (flag 4), a shifted key replaces the code and drops Shift, so
/// Ctrl-_ arrives as itself on any layout; a Ctrl or Alt chord on a
/// non-Latin layout takes the US-layout base key, so Ctrl-ц is Ctrl-w.
fn kitty(params: &[u8]) -> Option<Event> {
  let code = number(params, 0, 0)?;
  let (mut modifiers, mut state) = modifiers(number(params, 1, 0).unwrap_or(1));
  let kind = kind(number(params, 1, 1).unwrap_or(1));
  let mut code = match code {
    9 if modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
    9 => KeyCode::Tab,
    13 => KeyCode::Enter,
    27 => KeyCode::Esc,
    127 => KeyCode::Backspace,
    57399..=57427 => {
      state |= KeyEventState::KEYPAD;
      keypad(code)?
    }
    57358 => KeyCode::CapsLock,
    57359 => KeyCode::ScrollLock,
    57360 => KeyCode::NumLock,
    57361 => KeyCode::PrintScreen,
    57362 => KeyCode::Pause,
    57363 => KeyCode::Menu,
    57376..=57398 => KeyCode::F((code - 57376 + 13) as u8),
    // Other private-use codes are media and modifier keys.
    57344..=63743 => return None,
    _ => KeyCode::Char(char::from_u32(code)?),
  };
  if modifiers.contains(KeyModifiers::SHIFT)
    && let Some(shifted) = number(params, 0, 1).and_then(char::from_u32)
    && code != KeyCode::Char(shifted)
  {
    code = KeyCode::Char(shifted);
    modifiers -= KeyModifiers::SHIFT;
  }
  if modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    && matches!(code, KeyCode::Char(ch) if !ch.is_ascii())
    && let Some(base) = number(params, 0, 2).and_then(char::from_u32)
  {
    code = KeyCode::Char(base);
  }
  if (modifiers - KeyModifiers::SHIFT).is_empty()
    && kind != KeyEventKind::Release
    && matches!(code, KeyCode::Char(_))
    && let Some(text) = params.split(|&byte| byte == b';').nth(2)
  {
    let text: String = std::str::from_utf8(text)
      .ok()?
      .split(':')
      .map(|code| {
        code.parse().ok().and_then(char::from_u32).filter(|ch| !ch.is_control())
      })
      .collect::<Option<_>>()?;
    let mut chars = text.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
      return Some(Event::Paste(text));
    }
    code = KeyCode::Char(ch);
  }
  Some(Event::Key(KeyEvent::new_with_kind_and_state(
    code, modifiers, kind, state,
  )))
}

fn keypad(code: u32) -> Option<KeyCode> {
  Some(match code {
    57399..=57408 => KeyCode::Char(char::from_digit(code - 57399, 10)?),
    57409 => KeyCode::Char('.'),
    57410 => KeyCode::Char('/'),
    57411 => KeyCode::Char('*'),
    57412 => KeyCode::Char('-'),
    57413 => KeyCode::Char('+'),
    57414 => KeyCode::Enter,
    57415 => KeyCode::Char('='),
    57416 => KeyCode::Char(','),
    57417 => KeyCode::Left,
    57418 => KeyCode::Right,
    57419 => KeyCode::Up,
    57420 => KeyCode::Down,
    57421 => KeyCode::PageUp,
    57422 => KeyCode::PageDown,
    57423 => KeyCode::Home,
    57424 => KeyCode::End,
    57425 => KeyCode::Insert,
    57426 => KeyCode::Delete,
    _ => KeyCode::KeypadBegin,
  })
}

/// SGR `CSI < flags ; x ; y M` (press) or `m` (release). Flags: buttons
/// in bits 0-1 plus 64 for 4-7 and 128 for 8-11; 4 Shift, 8 Alt,
/// 16 Ctrl, 32 motion, 256 Kitty's leave.
fn mouse(params: &[u8], release: bool, pixels: bool) -> Option<Event> {
  let flags = number(params, 0, 0)?;
  // Drags past the window's top or left go negative.
  let coordinate = |index| match params.split(|&byte| byte == b';').nth(index) {
    Some([b'-', rest @ ..]) if pixels => number(rest, 0, 0).map(|_| 0),
    _ => number(params, index, 0),
  };
  let x = coordinate(1)?;
  let y = coordinate(2)?;
  let modifiers = held(
    flags,
    &[
      (4, KeyModifiers::SHIFT),
      (8, KeyModifiers::ALT),
      (16, KeyModifiers::CONTROL),
    ],
  );
  if flags & 256 != 0 {
    return Some(Event::Leave);
  }
  let at = if pixels {
    Spot::Pixel { x: small(x), y: small(y) }
  } else {
    Spot::Cell {
      column: small(x.saturating_sub(1)),
      row: small(y.saturating_sub(1)),
    }
  };
  let low = (flags & 3) as u8;
  let button = match flags & 0xc0 {
    0 if low < 3 => Some(Button(low + 1)),
    64 => Some(Button(low + 4)),
    128 => Some(Button(low + 8)),
    _ => None,
  };
  let motion = flags & 32 != 0;
  let kind = match (button, motion, release) {
    (Some(button), _, true) => MouseKind::Release(button),
    (Some(button), true, false) => MouseKind::Drag(button),
    (Some(button), false, false) => MouseKind::Press(button),
    (None, true, _) => MouseKind::Moved,
    (None, false, _) => return None,
  };
  Some(Event::Mouse(Mouse { kind, at, modifiers }))
}

/// OSC, DCS, APC, SOS, and PM strings, which end at ST; OSC also at BEL.
fn string(bytes: &[u8], resume: usize) -> Step {
  let osc = bytes[1] == b']';
  let from = resume.saturating_sub(1).max(2);
  let Some(offset) =
    bytes[from..].iter().position(|&byte| byte == 0x1b || (osc && byte == 7))
  else {
    return Step::Partial;
  };
  let end = from + offset;
  let used = match (bytes[end], bytes.get(end + 1)) {
    (7, _) => end + 1,
    (_, None) => return Step::Partial,
    (_, Some(b'\\')) => end + 2,
    // An ESC that starts something else abandons the string.
    (_, Some(_)) => return Step::Skip(end),
  };
  let content = &bytes[2..end];
  let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
  let reply = match bytes[1] {
    b']' => {
      let (code, body) = semicolon(content);
      number(code, 0, 0).map(|code| Reply::Osc { code, body: text(body) })
    }
    b'P' => content.strip_prefix(b">|").map(|name| Reply::Version(text(name))),
    b'_' => content.strip_prefix(b"G").map(|graphics| {
      let (keys, message) = semicolon(graphics);
      let value = |name: u8| {
        keys.split(|&byte| byte == b',').find_map(|pair| match pair {
          [key, b'=', value @ ..] if *key == name => number(value, 0, 0),
          _ => None,
        })
      };
      Reply::Graphics {
        id: value(b'i'),
        placement: value(b'p'),
        message: text(message),
      }
    }),
    _ => None,
  };
  match reply {
    Some(reply) => Step::Emit(Event::Reply(reply), used),
    None => Step::Skip(used),
  }
}

/// Splits at the first `;`.
fn semicolon(bytes: &[u8]) -> (&[u8], &[u8]) {
  match bytes.iter().position(|&byte| byte == b';') {
    Some(split) => (&bytes[..split], &bytes[split + 1..]),
    None => (bytes, &[]),
  }
}

#[cfg(test)]
mod tests;
