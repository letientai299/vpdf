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
    // Save the window title; Drop restores it.
    io::stdout().write_all(b"\x1b[22;0t")?;
    Ok(guard)
  }
}

impl Drop for Terminal {
  fn drop(&mut self) {
    let mut output = io::stdout();
    let _ = output.write_all(b"\x1b[?2026l\x1b[0m\x1b[23;0t");
    let _ = crate::graphics::release(&mut output);
    let _ = execute!(
      output,
      cursor::SetCursorStyle::DefaultUserShape,
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
    origin: pane_origin().unwrap_or_default(),
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

pub fn placement(
  output: &mut impl Write,
  control: &str,
  column: u32,
  row: u32,
) -> Result<()> {
  let sequence =
    format!("\x1b7\x1b[{row};{column}H\x1b_G{control};\x1b\\\x1b8");
  if std::env::var_os("TMUX").is_some() {
    write!(output, "\x1bPtmux;{}\x1b\\", sequence.replace('\x1b', "\x1b\x1b"))?;
  } else {
    write!(output, "{sequence}")?;
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
