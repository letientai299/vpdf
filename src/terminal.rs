use crate::Result;
use crate::layout::Screen;
use crossterm::{cursor, event, execute, terminal};
use std::io::{self, Write};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub struct Terminal;

impl Terminal {
  pub fn enter() -> Result<Self> {
    resume()?;
    // Save the window title; Drop restores it.
    io::stdout().write_all(b"\x1b[22;0t")?;
    Ok(Self)
  }
}

/// Whether the terminal speaks Kitty's keyboard protocol, which reports
/// keys such as Shift-Space that legacy encoding cannot tell apart.
fn enhanced() -> bool {
  static ENHANCED: OnceLock<bool> = OnceLock::new();
  *ENHANCED
    .get_or_init(|| terminal::supports_keyboard_enhancement().unwrap_or(false))
}

/// Set when another program had the screen; it may have deleted images.
static LOST: AtomicBool = AtomicBool::new(false);

/// Set while the reader holds the screen in raw mode.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether images need uploading again since the last call.
pub fn images_lost() -> bool {
  LOST.swap(false, Ordering::Relaxed)
}

/// Takes the terminal back after `suspend`.
pub fn resume() -> Result<()> {
  LOST.store(true, Ordering::Relaxed);
  ACTIVE.store(true, Ordering::SeqCst);
  terminal::enable_raw_mode()?;
  execute!(
    io::stdout(),
    terminal::EnterAlternateScreen,
    cursor::Hide,
    event::EnableMouseCapture,
    event::EnableBracketedPaste
  )?;
  if enhanced() {
    let flags = event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;
    execute!(io::stdout(), event::PushKeyboardEnhancementFlags(flags))?;
  }
  Ok(())
}

/// Hands the terminal to a child program; images stay cached.
pub fn suspend() -> Result<()> {
  // Leaving the alternate screen twice moves the cursor back.
  if !ACTIVE.swap(false, Ordering::SeqCst) {
    return Ok(());
  }
  if enhanced() {
    execute!(io::stdout(), event::PopKeyboardEnhancementFlags)?;
  }
  execute!(
    io::stdout(),
    cursor::SetCursorStyle::DefaultUserShape,
    event::DisableMouseCapture,
    event::DisableBracketedPaste,
    cursor::Show,
    terminal::LeaveAlternateScreen
  )?;
  terminal::disable_raw_mode()?;
  Ok(())
}

impl Drop for Terminal {
  fn drop(&mut self) {
    restore();
  }
}

/// Gives the screen back to the shell: ends any frame, restores the
/// title, deletes this reader's images, and leaves raw mode.
pub fn restore() {
  if !ACTIVE.load(Ordering::SeqCst) {
    return;
  }
  let mut output = io::stdout();
  let _ = output.write_all(b"\x1b[?2026l\x1b[0m\x1b[23;0t");
  let _ = crate::graphics::release(&mut output);
  let _ = suspend();
}

/// Restores the screen before a panic on the input thread prints, so the
/// message stays visible after the alternate screen closes.
pub fn restore_on_panic() {
  let default = std::panic::take_hook();
  std::panic::set_hook(Box::new(move |info| {
    if std::thread::current().name() == Some("main") {
      restore();
    }
    default(info);
  }));
}

pub fn size() -> Result<Screen> {
  let size = terminal::window_size()?;
  let columns = size.columns.max(1);
  let rows = size.rows.max(2);
  Ok(Screen {
    columns,
    rows,
    origin: pane_origin().unwrap_or_default(),
    cell_width: if size.width == 0 { 8 } else { (size.width / columns).max(1) },
    cell_height: if size.height == 0 {
      16
    } else {
      (size.height / rows).max(1)
    },
  })
}

/// Whether output passes through tmux; read once per process.
fn tmux() -> bool {
  static TMUX: OnceLock<bool> = OnceLock::new();
  *TMUX.get_or_init(|| std::env::var_os("TMUX").is_some())
}

pub fn graphics(
  output: &mut impl Write,
  control: &str,
  data: &str,
) -> Result<()> {
  if tmux() {
    write!(output, "\x1bPtmux;\x1b\x1b_G{control};{data}\x1b\x1b\\\x1b\\")?;
  } else {
    write!(output, "\x1b_G{control};{data}\x1b\\")?;
  }
  Ok(())
}

/// `text` without control characters, cut to `columns` cells.
pub fn clean(text: &str, columns: usize) -> String {
  let mut width = 0;
  let mut clean: String = text
    .chars()
    .filter(|ch| !ch.is_control())
    .take_while(|ch| {
      width += ch.width().unwrap_or(0);
      width <= columns
    })
    .collect();
  // Sequences such as an emoji with U+FE0F are wider than their chars.
  while clean.width() > columns {
    clean.pop();
  }
  clean
}

/// Sends an encoded image in 4096-byte chunks; `control` leads the first.
pub fn transmit(
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
      graphics(output, &format!("{control},m={}", u8::from(more)), chunk)?;
      first = false;
    } else {
      graphics(output, if more { "m=1,q=2" } else { "m=0,q=2" }, chunk)?;
    }
  }
  Ok(())
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
  // Cursor save, move, placement, and restore travel in one packet.
  if tmux() {
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

fn pane_origin() -> Option<(u16, u16)> {
  let pane = std::env::var_os("TMUX_PANE")?;
  let output = std::process::Command::new("tmux")
    .args(["display-message", "-p", "-t"])
    .arg(pane)
    .arg("#{pane_left} #{pane_top} #{status-position} #{status}")
    .stdin(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .output()
    .ok()?;
  if !output.status.success() {
    return None;
  }
  let text = std::str::from_utf8(&output.stdout).ok()?;
  let mut fields = text.split_whitespace();
  let column: u16 = fields.next()?.parse().ok()?;
  let mut row: u16 = fields.next()?.parse().ok()?;
  if fields.next()? == "top" {
    let rows = match fields.next()? {
      "on" => 1,
      "off" => 0,
      value => value.parse().ok()?,
    };
    row = row.saturating_add(rows);
  }
  Some((column, row))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn clean_fits_string_width() {
    assert_eq!(clean("a\tb", 5), "ab");
    // Each heart with U+FE0F is two cells, though its chars sum to one.
    let hearts = "❤\u{FE0F}".repeat(4);
    for columns in 0..8 {
      let clean = clean(&hearts, columns);
      assert!(hearts.starts_with(&clean));
      assert!(clean.width() <= columns, "{columns}");
    }
  }
}
