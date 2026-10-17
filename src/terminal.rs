use crate::Result;
use crate::caps::{Caps, Cell, Host, Support};
use crate::decode::Size;
use crate::layout::Screen;
use anyhow::Context;
use crossterm::terminal::WindowSize;
use crossterm::{cursor, event, execute, terminal};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub struct Terminal;

impl Terminal {
  pub fn enter() -> Result<Self> {
    crate::input::start()?;
    // Built first, so a failed probe still restores the screen.
    let terminal = Self;
    resume()?;
    // Save the window title; Drop restores it.
    io::stdout().write_all(b"\x1b[22;0t")?;
    Ok(terminal)
  }
}

/// Whether the terminal speaks Kitty's keyboard protocol, which reports
/// keys such as Shift-Space that legacy encoding cannot tell apart.
fn enhanced() -> bool {
  crate::caps::get().is_some_and(|caps| caps.keyboard.is_some())
}

/// Set when another program had the screen; it may have deleted images.
static LOST: AtomicBool = AtomicBool::new(false);

/// Set while the reader holds the screen in raw mode.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether images need uploading again since the last call.
pub fn images_lost() -> bool {
  LOST.swap(false, Ordering::Relaxed)
}

/// Set when a prompt outside the event loop read a resize; only the
/// session loop clears it.
static RESIZED: AtomicBool = AtomicBool::new(false);

/// Passes a resize a prompt read on to the event loop.
pub fn defer_resize(report: Option<Size>) {
  if let Some(report) = report {
    PROVOKED.store(false, Ordering::Relaxed);
    note(report);
  }
  RESIZED.store(true, Ordering::Relaxed);
}

/// Whether a deferred resize waits since the last call.
pub fn resize_deferred() -> bool {
  RESIZED.swap(false, Ordering::Relaxed)
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
  // Inside the alternate screen, so a terminal that prints an unknown
  // query leaves nothing behind.
  crate::caps::probe()?;
  resize_reports(true)?;
  if enhanced() {
    let flags = event::KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
      | event::KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;
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
  crate::input::forget_answers();
  resize_reports(false)?;
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
  let _ = output.flush();
  crate::spool::remove();
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

/// Whether the terminal sends mode 2048 resize reports, which carry
/// the pixel size where TIOCGWINSZ may have none. While they are on,
/// each resize also sends one, so SIGWINCH adds nothing.
fn reports() -> bool {
  crate::caps::get().is_some_and(|caps| caps.resize == Support::Answered)
}

/// Turns mode 2048 resize reports on or off where the terminal has
/// them; turning them on also reports the current size.
fn resize_reports(on: bool) -> Result<()> {
  if reports() {
    PROVOKED.store(on, Ordering::Relaxed);
    let mut out = io::stdout();
    out.write_all(if on { b"\x1b[?2048h" } else { b"\x1b[?2048l" })?;
    out.flush()?;
  }
  Ok(())
}

/// Set while the report that turning reports on provokes is due.
static PROVOKED: AtomicBool = AtomicBool::new(false);

pub const NO_CELL: &str = "Terminal reports no cell size in pixels";

/// What tmux puts in a pane's pixel size when it does not know the
/// client's cell size ([tmux.h][h]).
///
/// [h]: https://github.com/tmux/tmux/blob/master/tmux.h
const TMUX_FAKE: Cell = Cell { width: 16, height: 32 };

/// The latest cell size the terminal told in band, by a mode 2048
/// report or a `CSI 16 t` answer after a resize.
static TOLD: Mutex<Option<Cell>> = Mutex::new(None);

fn told() -> Option<Cell> {
  *TOLD.lock().unwrap_or_else(PoisonError::into_inner)
}

fn tell(cell: Cell) {
  *TOLD.lock().unwrap_or_else(PoisonError::into_inner) = Some(cell);
}

fn window(report: Size) -> WindowSize {
  WindowSize {
    rows: report.rows,
    columns: report.columns,
    width: report.width,
    height: report.height,
  }
}

/// The cell size `window`'s pixels imply; None without pixels.
fn window_cell(window: &WindowSize) -> Option<Cell> {
  let part = |pixels: u16, cells: u16| {
    (pixels > 0 && cells > 0).then(|| (pixels / cells).max(1))
  };
  Some(Cell {
    width: part(window.width, window.columns)?,
    height: part(window.height, window.rows)?,
  })
}

/// Picks the cell size: the window's pixels, except tmux's made-up
/// 16x32, then under tmux the client's cell as tmux knows it now;
/// elsewhere the latest in-band answer, then the probe's `CSI 16 t`.
/// None rather than a guess. `tmux` stands in for `caps` before the
/// probe.
fn pick(
  window: Option<Cell>,
  told: Option<Cell>,
  caps: Option<&Caps>,
  tmux: bool,
  client: impl FnOnce() -> Option<Cell>,
) -> Option<Cell> {
  let tmux = caps.map_or(tmux, |caps| matches!(caps.host, Host::Tmux(_)));
  if tmux {
    return window.filter(|&cell| cell != TMUX_FAKE).or_else(client);
  }
  window.or(told).or(caps.and_then(|caps| caps.cell))
}

fn metrics(window: &WindowSize) -> Option<Cell> {
  let client = || crate::caps::tmux_now()?.cell;
  pick(window_cell(window), told(), crate::caps::get(), tmux(), client)
}

/// The cell size in pixels, without asking the terminal; None when
/// nothing reports it.
pub fn cell() -> Option<Cell> {
  metrics(&terminal::window_size().ok()?)
}

/// The screen as TIOCGWINSZ and earlier answers describe it; an error
/// when nothing reports the cell size in pixels.
pub fn size() -> Result<Screen> {
  screen(&terminal::window_size()?)
}

/// Keeps the cell a mode 2048 report carries for later sizes.
fn note(report: Size) {
  if let Some(cell) = window_cell(&window(report)) {
    tell(cell);
  }
}

/// Whether a SIGWINCH should ask `CSI 16 t` again: only without
/// pixels, and not while a later resize waits in the queue.
fn asks_again(window: Option<Cell>, queued: bool) -> bool {
  window.is_none() && !queued
}

/// The screen a resize event brings, from its mode 2048 `report` when
/// one came; None when the event changes nothing: SIGWINCH while
/// reports carry the size, or the report that turning reports on
/// provokes at the `current` size.
pub fn resized(
  report: Option<Size>,
  current: Option<Screen>,
) -> Result<Option<Screen>> {
  let Some(report) = report else {
    return if reports() { Ok(None) } else { remeasure().map(Some) };
  };
  let provoked = PROVOKED.swap(false, Ordering::Relaxed);
  note(report);
  let screen = screen(&window(report))?;
  Ok((!provoked || current != Some(screen)).then_some(screen))
}

/// The screen after a resize without its report, as SIGWINCH or a
/// deferred resize leaves it: asks `CSI 16 t` again when needed.
pub fn remeasure() -> Result<Screen> {
  let window = terminal::window_size()?;
  let queued = crate::input::resizing();
  if !reports()
    && asks_again(window_cell(&window), queued)
    && let Some(cell) = crate::caps::ask_cell()
  {
    tell(cell);
  }
  screen(&window)
}

fn screen(window: &WindowSize) -> Result<Screen> {
  let cell = metrics(window).context(NO_CELL)?;
  Ok(Screen {
    columns: window.columns.max(1),
    rows: window.rows.max(2),
    origin: pane_origin().unwrap_or_default(),
    cell_width: cell.width,
    cell_height: cell.height,
  })
}

/// Whether output passes through tmux; read once per process.
pub fn tmux() -> bool {
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

/// `format` expanded by tmux for vpdf's pane; None outside tmux or
/// when tmux fails.
pub fn tmux_display(format: &str) -> Option<String> {
  let pane = std::env::var_os("TMUX_PANE")?;
  let output = std::process::Command::new("tmux")
    .args(["display-message", "-p", "-t"])
    .arg(pane)
    .arg(format)
    .stdin(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .output()
    .ok()?;
  if !output.status.success() {
    return None;
  }
  String::from_utf8(output.stdout).ok()
}

fn pane_origin() -> Option<(u16, u16)> {
  let text =
    tmux_display("#{pane_left} #{pane_top} #{status-position} #{status}")?;
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
  fn cell_size_never_guesses() {
    use crate::caps::{Graphics, Tmux};
    let cell = |width, height| Some(Cell { width, height });
    let caps = |host, asked| Caps {
      host,
      version: None,
      keyboard: None,
      sync: Support::Missing,
      pixels: Support::Missing,
      resize: Support::Missing,
      pointer: Support::Missing,
      drop: Support::Missing,
      cell: asked,
      attributes: Some(vec![62]),
    };
    let direct = caps(Host::Direct(Graphics::Ok), cell(9, 19));
    let unasked = caps(Host::Direct(Graphics::Ok), None);
    // The probe's client cell is stale; tmux is asked again.
    let tmux = caps(
      Host::Tmux(Some(Tmux {
        version: vec![3, 8],
        client: None,
        cell: cell(5, 11),
        passthrough: None,
      })),
      None,
    );
    let failed = caps(Host::Tmux(None), None);
    let fake = Some(TMUX_FAKE);
    // (window, told, caps, TMUX set, client now, want)
    let cases = [
      (cell(8, 17), cell(10, 20), Some(&direct), false, None, cell(8, 17)),
      (None, cell(10, 20), Some(&direct), false, None, cell(10, 20)),
      (None, None, Some(&direct), false, None, cell(9, 19)),
      (None, None, Some(&unasked), false, None, None),
      (fake, None, Some(&direct), false, None, fake),
      (cell(7, 13), None, Some(&tmux), false, cell(6, 12), cell(7, 13)),
      (fake, None, Some(&tmux), false, cell(7, 13), cell(7, 13)),
      (fake, None, Some(&tmux), false, fake, fake),
      (fake, cell(10, 20), Some(&tmux), false, None, None),
      (None, None, Some(&tmux), false, cell(7, 13), cell(7, 13)),
      (fake, None, Some(&failed), true, None, None),
      (fake, None, None, true, cell(7, 13), cell(7, 13)),
      (fake, None, None, true, None, None),
      (cell(9, 18), None, None, true, None, cell(9, 18)),
      (fake, None, None, false, cell(7, 13), fake),
      (None, None, None, false, None, None),
    ];
    for (index, (window, told, caps, tmux, client, want)) in
      cases.into_iter().enumerate()
    {
      let got = pick(window, told, caps, tmux, || client);
      assert_eq!(got, want, "case {index}");
    }
  }

  #[test]
  fn resizes_ask_again_without_pixels() {
    let some = Some(Cell { width: 8, height: 16 });
    // (window cell, resize queued, want)
    let cases =
      [(None, false, true), (None, true, false), (some, false, false)];
    for (window, queued, want) in cases {
      assert_eq!(asks_again(window, queued), want, "{window:?} {queued}");
    }
  }

  #[test]
  fn window_pixels_divide_into_cells() {
    let window = |columns, rows, width, height| WindowSize {
      rows,
      columns,
      width,
      height,
    };
    let cases = [
      (window(80, 24, 640, 384), Some(Cell { width: 8, height: 16 })),
      (window(3, 2, 2, 1), Some(Cell { width: 1, height: 1 })),
      (window(80, 24, 0, 384), None),
      (window(80, 24, 640, 0), None),
      (window(0, 24, 640, 384), None),
    ];
    for (window, want) in cases {
      assert_eq!(window_cell(&window), want, "{window:?}");
    }
  }

  #[test]
  fn reports_keep_their_cell() {
    let report = Size { rows: 24, columns: 80, height: 480, width: 720 };
    note(report);
    assert_eq!(told(), Some(Cell { width: 9, height: 20 }));
  }

  #[test]
  fn clean_fits_string_width() {
    assert_eq!(clean("a\tb\x1b", 5), "a b");
    // Each heart with U+FE0F is two cells, though its chars sum to one.
    let hearts = "❤\u{FE0F}".repeat(4);
    for columns in 0..8 {
      let clean = clean(&hearts, columns);
      assert!(hearts.starts_with(&clean));
      assert!(clean.width() <= columns, "{columns}");
    }
    let hostile = hearts.repeat(25) + &"\u{301}".repeat(50_000);
    let clean = clean(&hostile, 160);
    assert!(clean.width() <= 160 && clean.chars().count() <= 8 * 160);
  }
}
