use crate::Result;
use crate::layout::Screen;
use crossterm::{cursor, event, execute, terminal};
use std::io::{self, Write};
use unicode_width::UnicodeWidthChar;

pub struct Terminal;

impl Terminal {
  pub fn enter() -> Result<Self> {
    terminal::enable_raw_mode()?;
    let guard = Self;
    execute!(
      io::stdout(),
      terminal::EnterAlternateScreen,
      cursor::Hide,
      event::EnableMouseCapture,
      event::EnableBracketedPaste
    )?;
    Ok(guard)
  }
}

impl Drop for Terminal {
  fn drop(&mut self) {
    let mut output = io::stdout();
    let _ = output.write_all(b"\x1b[?2026l\x1b[0m");
    let _ = graphics(&mut output, "a=d,d=A,q=2", "");
    let _ = execute!(
      output,
      event::DisableMouseCapture,
      event::DisableBracketedPaste,
      cursor::Show,
      terminal::LeaveAlternateScreen
    );
    let _ = terminal::disable_raw_mode();
  }
}

pub fn size() -> Result<Screen> {
  let size = terminal::window_size()?;
  let columns = size.columns.max(1);
  let rows = size.rows.max(2);
  Ok(Screen {
    columns,
    rows,
    cell_width: if size.width == 0 { 8 } else { (size.width / columns).max(1) },
    cell_height: if size.height == 0 {
      16
    } else {
      (size.height / rows).max(1)
    },
  })
}

pub fn graphics(
  output: &mut impl Write,
  control: &str,
  data: &str,
) -> Result<()> {
  if std::env::var_os("TMUX").is_some() {
    write!(output, "\x1bPtmux;\x1b\x1b_G{control};{data}\x1b\x1b\\\x1b\\")?;
  } else {
    write!(output, "\x1b_G{control};{data}\x1b\\")?;
  }
  Ok(())
}

pub fn clean(text: &str, columns: usize) -> String {
  let mut width = 0;
  text
    .chars()
    .filter(|ch| !ch.is_control())
    .take_while(|ch| {
      width += ch.width().unwrap_or(0);
      width <= columns
    })
    .collect()
}

pub fn line(
  output: &mut impl Write,
  row: u16,
  text: &str,
  columns: u16,
) -> Result<()> {
  write!(
    output,
    "\x1b[{};1H\x1b[2K{}",
    row + 1,
    clean(text, usize::from(columns))
  )?;
  Ok(())
}
