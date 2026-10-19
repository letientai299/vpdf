use super::{
  Caps, Host, Result, UnicodeWidthChar, UnicodeWidthStr, Write, tmux,
};

/// Hides the text cursor, which frames move.
pub const HIDE_CURSOR: &str = "\x1b[?25l";

/// Sets the window title, except where the terminal ignores the title
/// stack and would keep it after exit.
pub fn set_title(out: &mut impl Write, title: &str) -> Result<()> {
  if crate::caps::get().is_none_or(Caps::title_stack) {
    write!(out, "\x1b]2;{}\x07", clean(title, 200))?;
  }
  Ok(())
}

/// Starts a frame the terminal shows whole (mode 2026), except under
/// tmux before 3.8: it sends images through at once, ahead of the held
/// text, and 3.7b and 3.7c drop the next image after the frame ends.
pub fn frame_start() -> &'static str {
  if synced(crate::caps::get()) { "\x1b[?2026h" } else { "" }
}

/// Ends a frame `frame_start` began.
pub fn frame_end() -> &'static str {
  if synced(crate::caps::get()) { "\x1b[?2026l" } else { "" }
}

pub(super) fn synced(caps: Option<&Caps>) -> bool {
  caps.is_none_or(|caps| {
    !matches!(&caps.host, Host::Tmux(Some(tmux)) if tmux.before(&[3, 8]))
  })
}

pub fn passthrough(
  output: &mut impl Write,
  bytes: &[u8],
  tmux: bool,
) -> Result<()> {
  if !tmux {
    output.write_all(bytes)?;
    return Ok(());
  }
  output.write_all(b"\x1bPtmux;")?;
  for part in bytes.split_inclusive(|&byte| byte == 0x1b) {
    output.write_all(part)?;
    if part.last() == Some(&0x1b) {
      output.write_all(b"\x1b")?;
    }
  }
  output.write_all(b"\x1b\\")?;
  Ok(())
}

pub fn graphics(
  output: &mut impl Write,
  control: &str,
  data: &str,
) -> Result<()> {
  Transport::live().graphics(output, control, data)
}

/// `text` made printable, cut to `columns` cells.
pub fn clean(text: &str, columns: usize) -> String {
  let mut width = 0;
  let mut clean: String = crate::text::printable(text)
    // Caps zero-width marks from hostile text.
    .take(columns.saturating_mul(8))
    .take_while(|ch| {
      width += ch.width().unwrap_or(0);
      width <= columns
    })
    .collect();
  // Sequences such as an emoji with U+FE0F are wider than their chars.
  while clean.width() > columns {
    // Marks go with their base char.
    while clean.pop().is_some_and(|ch| ch.width() == Some(0)) {}
  }
  clean
}

/// `text` within `columns`, ending in `…` when cut; whether it was.
pub fn clipped(text: &str, columns: usize) -> (String, bool) {
  let clean = clean(text, columns);
  if clean.width() < text.width() && columns > 0 {
    (format!("{}…", self::clean(text, columns - 1)), true)
  } else {
    (clean, false)
  }
}

/// Sends an encoded image in 4096-byte chunks; `control` leads the first.
pub fn transmit(
  output: &mut impl Write,
  control: &str,
  encoded: &str,
) -> Result<()> {
  Transport::live().transmit(output, control, encoded)
}

/// A pane's top-left cell on the terminal; drawing positions add it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct At {
  pub column: u16,
  pub row: u16,
}

impl At {
  /// Cursor move to zero-based `row` and `column` inside the pane.
  pub fn goto(self, row: impl Into<usize>, column: impl Into<usize>) -> String {
    format!(
      "\x1b[{};{}H",
      row.into() + usize::from(self.row) + 1,
      column.into() + usize::from(self.column) + 1
    )
  }
}

/// Writes `text` on a pane row, padded so only the pane's cells change.
pub fn line(
  output: &mut impl Write,
  at: At,
  row: u16,
  text: &str,
  columns: u16,
) -> Result<()> {
  let text = clean(text, usize::from(columns));
  let pad = usize::from(columns).saturating_sub(text.width());
  write!(output, "{}{text}{}", at.goto(row, 0u16), " ".repeat(pad))?;
  Ok(())
}

/// Blanks a pane row by erasing its cells, which costs a few bytes
/// where spaces cost one per column.
pub fn clear(
  output: &mut impl Write,
  at: At,
  row: u16,
  columns: u16,
) -> Result<()> {
  write!(output, "{}\x1b[{columns}X", at.goto(row, 0u16))?;
  Ok(())
}

pub fn placement(
  output: &mut impl Write,
  control: &str,
  column: u32,
  row: u32,
) -> Result<()> {
  Transport::live().placement(output, control, column, row)
}

/// Wire framing, independent of live negotiation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Transport {
  pub tmux: bool,
}

impl Transport {
  pub fn live() -> Self {
    Self { tmux: tmux() }
  }
  pub fn graphics(
    self,
    output: &mut impl Write,
    control: &str,
    data: &str,
  ) -> Result<()> {
    if self.tmux {
      write!(output, "\x1bPtmux;\x1b\x1b_G{control};{data}\x1b\x1b\\\x1b\\")?;
    } else {
      write!(output, "\x1b_G{control};{data}\x1b\\")?;
    }
    Ok(())
  }

  pub fn transmit(
    self,
    output: &mut impl Write,
    control: &str,
    encoded: &str,
  ) -> Result<()> {
    let mut chunks = encoded.as_bytes().chunks(4096).peekable();
    let mut first = true;
    while let Some(chunk) = chunks.next() {
      let more = chunks.peek().is_some();
      let chunk = std::str::from_utf8(chunk)?;
      if first {
        self.graphics(
          output,
          &format!("{control},m={}", u8::from(more)),
          chunk,
        )?;
        first = false;
      } else {
        self.graphics(
          output,
          if more { "m=1,q=2" } else { "m=0,q=2" },
          chunk,
        )?;
      }
    }
    Ok(())
  }

  pub fn placement(
    self,
    output: &mut impl Write,
    control: &str,
    column: u32,
    row: u32,
  ) -> Result<()> {
    // Cursor save, move, placement, and restore travel in one packet.
    if self.tmux {
      write!(
        output,
        "\x1bPtmux;\x1b\x1b7\x1b\x1b[{row};{column}H\
       \x1b\x1b_G{control};\x1b\x1b\\\x1b\x1b8\x1b\\"
      )?;
    } else {
      write!(output, "\x1b7\x1b[{row};{column}H\x1b_G{control};\x1b\\\x1b8")?;
    }
    Ok(())
  }
}
