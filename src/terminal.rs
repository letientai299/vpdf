use crate::Result;
use crate::caps::{Caps, Cell, Host, Support};
use crate::decode::Size;
use crate::input::Keyboard;
use crate::layout::Screen;
use anyhow::Context;
use crossterm::terminal::WindowSize;
use crossterm::{cursor, event, execute, terminal};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once, PoisonError};
use std::time::{Duration, Instant};
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

/// Set when another program had the screen; it may have deleted images.
static LOST: AtomicBool = AtomicBool::new(false);

/// Set while the reader holds the screen in raw mode.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Set while the reader's Kitty keyboard flags sit on the terminal's
/// stack, or under tmux its modifyOtherKeys mode, so suspend undoes
/// exactly what resume set.
static PUSHED: AtomicBool = AtomicBool::new(false);

/// Set while the terminal sends drops, OSC 72.
static DROPS: AtomicBool = AtomicBool::new(false);

/// Set while mouse reports carry pixels, mode 1016.
static PIXELS: AtomicBool = AtomicBool::new(false);

/// Set while tmux sends the pane focus events, mode 1004, by which the
/// session hides placements tmux would leave over another window.
static FOCUS: AtomicBool = AtomicBool::new(false);

/// Pointer shape the reader last wrote.
static SHAPE: Mutex<Shapes> = Mutex::new(Shapes { owner: None, shown: None });

/// OSC 22 pointer shapes ([spec][spec]).
///
/// [spec]: https://sw.kovidgoyal.net/kitty/pointer-shapes/
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Shape {
  #[default]
  Default,
  Pointer,
  Text,
}

impl Shape {
  fn name(self) -> &'static str {
    match self {
      Self::Default => "default",
      Self::Pointer => "pointer",
      Self::Text => "text",
    }
  }
}

/// How the reader holds the pointer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Owner {
  /// Kitty: pushed on its shape stack.
  Stack,
  /// Ghostty: set by name, starting at `text`.
  Named,
}

#[derive(Debug)]
struct Shapes {
  /// None without shapes or the screen.
  owner: Option<Owner>,
  /// None when unknown: the next shape writes.
  shown: Option<Shape>,
}

impl Shapes {
  /// Takes over the pointer where supported.
  fn push(
    &mut self,
    support: Support,
    out: &mut impl Write,
    tmux: bool,
  ) -> Result<()> {
    if self.owner.is_some() {
      return Ok(());
    }
    let (owner, body) = match support {
      Support::Answered => (Owner::Stack, ">default"),
      Support::Named => (Owner::Named, "default"),
      Support::Missing => return Ok(()),
    };
    osc22(out, body, tmux)?;
    self.owner = Some(owner);
    self.shown = Some(Shape::Default);
    Ok(())
  }

  /// Writes `shape` when it changed.
  fn point(
    &mut self,
    shape: Shape,
    out: &mut impl Write,
    tmux: bool,
  ) -> Result<()> {
    if self.owner.is_none() || self.shown == Some(shape) {
      return Ok(());
    }
    osc22(out, shape.name(), tmux)?;
    self.shown = Some(shape);
    Ok(())
  }

  /// Another program may have set one.
  fn forget(&mut self) {
    self.shown = None;
  }

  /// Restores the terminal's own shape.
  fn pop(&mut self, out: &mut impl Write, tmux: bool) -> Result<()> {
    self.shown = None;
    match self.owner.take() {
      Some(Owner::Stack) => osc22(out, "<", tmux),
      // Ghostty 1.3 ignores an empty reset.
      Some(Owner::Named) => osc22(out, Shape::Text.name(), tmux),
      None => Ok(()),
    }
  }
}

fn osc22(out: &mut impl Write, body: &str, tmux: bool) -> Result<()> {
  passthrough(out, format!("\x1b]22;{body}\x1b\\").as_bytes(), tmux)
}

fn shapes() -> std::sync::MutexGuard<'static, Shapes> {
  SHAPE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Sets the pointer shape; writes only changes.
pub fn point(out: &mut impl Write, shape: Shape) -> Result<()> {
  shapes().point(shape, out, tmux())
}

/// Sets `default` once; the next shape always writes.
pub fn hand_over(out: &mut impl Write) -> Result<()> {
  let mut shapes = shapes();
  if shapes.shown.is_none() {
    return Ok(());
  }
  shapes.point(Shape::Default, out, tmux())?;
  shapes.forget();
  Ok(())
}

/// Arms frames as a Kitty reader.
#[cfg(test)]
pub(crate) fn arm_shapes() {
  *shapes() = Shapes { owner: Some(Owner::Stack), shown: Some(Shape::Default) };
}

/// Lines the status line shows once, such as tmux settings vpdf changed
/// or lacks.
static NOTICES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The next line the status line should say once; None when nothing
/// waits.
pub fn notice() -> Option<String> {
  let mut notices = NOTICES.lock().unwrap_or_else(PoisonError::into_inner);
  (!notices.is_empty()).then(|| notices.remove(0))
}

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
  let caps = crate::caps::probe()?;
  resize_reports(true)?;
  let lack = caps.negotiate(|request| {
    PUSHED.store(true, Ordering::SeqCst);
    crate::input::ask(&mut io::stdout(), request, crate::caps::WAIT)
  })?;
  // Legacy bytes merge Ctrl-i with Tab and make Esc a prefix, so no
  // fallback. Input fails from now on, even where a caller of a resume
  // after a child program shows the error and goes on.
  if let Some(lack) = lack {
    crate::input::set_keyboard(Keyboard::Failed(lack.clone()));
    return Err(lack.into());
  }
  if caps.pixel_mouse() {
    // The parser first, so no pixel report reads as cells.
    crate::input::set_pixels(true);
    PIXELS.store(true, Ordering::Relaxed);
    let mut out = io::stdout();
    out.write_all(b"\x1b[?1016h")?;
    // Reports switch now, not at the first frame.
    out.flush()?;
  }
  shapes().push(caps.pointer, &mut io::stdout(), tmux())?;
  if caps.drop == Support::Answered {
    DROPS.store(true, Ordering::Relaxed);
    io::stdout().write_all(crate::dnd::ACCEPT)?;
  }
  match &caps.host {
    Host::Tmux(Some(tmux)) => tmux_setup(caps, tmux),
    _ => {
      crate::input::set_keyboard(Keyboard::Disambiguated);
      Ok(())
    }
  }
}

/// Under tmux: keeps the lone-ESC wait, since extended keys leave Esc a
/// lone byte; asks for focus events where tmux sends them; and on the
/// first call sets passthrough and queues the notices.
fn tmux_setup(caps: &Caps, tmux: &crate::caps::Tmux) -> Result<()> {
  static NOTED: Once = Once::new();
  crate::input::set_keyboard(Keyboard::Legacy);
  if tmux.focus_events {
    FOCUS.store(true, Ordering::Relaxed);
    io::stdout().write_all(b"\x1b[?1004h")?;
  }
  NOTED.call_once(|| {
    let notices = tmux_notices(caps, crate::caps::allow_passthrough());
    *NOTICES.lock().unwrap_or_else(PoisonError::into_inner) = notices;
  });
  Ok(())
}

/// What the status line says once under tmux: `passthrough`, the
/// setting vpdf changed or could not, then what the setup lacks.
fn tmux_notices(caps: &Caps, passthrough: Option<String>) -> Vec<String> {
  let mut notices = Vec::from_iter(passthrough);
  if matches!(&caps.host, Host::Tmux(Some(tmux)) if !tmux.focus_events) {
    notices.push(
      "tmux focus-events off: images, shapes outlast window switches".into(),
    );
  }
  if caps.merges_ctrl_i() {
    notices.push("Ctrl-i is Tab under tmux in Kitty".into());
  }
  notices
}

/// Hands the terminal to a child program; images stay cached, except
/// under tmux, whose alternate screen is its own: the terminal would
/// keep showing them over the child. Resume uploads them again.
pub fn suspend() -> Result<()> {
  // Leaving the alternate screen twice moves the cursor back.
  if !ACTIVE.swap(false, Ordering::SeqCst) {
    return Ok(());
  }
  if tmux() {
    crate::graphics::release(&mut io::stdout())?;
  }
  if DROPS.swap(false, Ordering::Relaxed) {
    io::stdout().write_all(crate::dnd::REFUSE)?;
  }
  // Before the parser reads cells again.
  if PIXELS.swap(false, Ordering::Relaxed) {
    io::stdout().write_all(b"\x1b[?1016l")?;
  }
  shapes().pop(&mut io::stdout(), tmux())?;
  // Before the keyboard pop, so a lone ESC is Esc.
  crate::input::release();
  resize_reports(false)?;
  if PUSHED.swap(false, Ordering::SeqCst) {
    // tmux takes modifyOtherKeys back to its default; others pop flags.
    let undo: &[u8] = if tmux() { b"\x1b[>4m" } else { b"\x1b[<u" };
    io::stdout().write_all(undo)?;
  }
  if FOCUS.swap(false, Ordering::Relaxed) {
    io::stdout().write_all(b"\x1b[?1004l")?;
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
  let _ = write!(output, "{}\x1b[0m\x1b[23;0t", frame_end());
  // Under tmux suspend deletes them.
  if !tmux() {
    let _ = crate::graphics::release(&mut output);
  }
  let _ = output.flush();
  crate::spool::remove();
  let _ = suspend();
  // After suspend's deletes, which a hidden pane sends only with `all`.
  crate::caps::reset_passthrough();
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

fn metrics(
  window: &WindowSize,
  client: impl FnOnce() -> Option<Cell>,
) -> Option<Cell> {
  pick(window_cell(window), told(), crate::caps::get(), tmux(), client)
}

/// The cell size in pixels, without asking the terminal; None when
/// nothing reports it.
pub fn cell() -> Option<Cell> {
  metrics(&terminal::window_size().ok()?, || crate::caps::tmux_now()?.cell)
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
  // One `tmux display` gives the pane's origin and the client's cell.
  let now = if tmux() { crate::caps::tmux_now() } else { None };
  let client = now.as_ref().and_then(|tmux| tmux.cell);
  let cell = metrics(window, || client).context(NO_CELL)?;
  Ok(Screen {
    columns: window.columns.max(1),
    rows: window.rows.max(2),
    origin: now.map_or_default(|tmux| tmux.origin),
    cell_width: cell.width,
    cell_height: cell.height,
  })
}

/// Whether output passes through tmux, as the probe routes it.
pub fn tmux() -> bool {
  crate::caps::under_tmux()
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

fn synced(caps: Option<&Caps>) -> bool {
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
  tmux_pane(&["display-message", "-p"], &[format])
}

/// Sets tmux option `name` for vpdf's pane; whether tmux took it.
pub fn tmux_set(name: &str, value: &str) -> bool {
  tmux_pane(&["set-option", "-p"], &[name, value]).is_some()
}

/// What tmux shows of vpdf's pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum View {
  /// No client shows its window, or another pane is zoomed.
  Hidden,
  /// Shown, while another pane of its window takes keys.
  Inactive,
  /// The pane that takes keys; whether the client has the OS focus.
  Active { focused: bool },
}

/// `View` as tmux evaluates it: shown when a client's current window is
/// the pane's and no other pane is zoomed ([formats][formats]).
///
/// [formats]: https://github.com/tmux/tmux/blob/3.8/format.c#L790-L820
const VIEW: &str = "#{?#{&&:#{window_active_clients},\
  #{||:#{pane_active},#{?window_zoomed_flag,0,1}}},\
  #{?pane_active,#{?#{m:*focused*,#{client_flags}},focused,away},inactive},\
  hidden}";

/// What tmux shows of vpdf's pane now; None outside tmux or when tmux
/// fails.
pub fn pane_view() -> Option<View> {
  view(tmux_display(VIEW)?.trim())
}

fn view(word: &str) -> Option<View> {
  Some(match word {
    "hidden" => View::Hidden,
    "inactive" => View::Inactive,
    "focused" => View::Active { focused: true },
    "away" => View::Active { focused: false },
    _ => return None,
  })
}

/// Whether tmux sends vpdf focus events, mode 1004.
pub fn focus_reports() -> bool {
  FOCUS.load(Ordering::Relaxed)
}

/// How long a `tmux` command may take; a `tmux` that does not match the
/// server can hang.
const TMUX_WAIT: Duration = Duration::from_secs(1);

/// The output of tmux `command` aimed at vpdf's pane, then `args`; None
/// outside tmux, or when tmux fails or takes over `TMUX_WAIT`.
fn tmux_pane(command: &[&str], args: &[&str]) -> Option<String> {
  let pane = std::env::var_os("TMUX_PANE")?;
  let mut child = std::process::Command::new("tmux")
    .args(command)
    .arg("-t")
    .arg(pane)
    .args(args)
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::null())
    .spawn()
    .ok()?;
  let deadline = Instant::now() + TMUX_WAIT;
  // Output fits the pipe, so the child exits before it is read.
  let status = loop {
    match child.try_wait() {
      Ok(Some(status)) => break status,
      Ok(None) if Instant::now() < deadline => {
        std::thread::sleep(Duration::from_millis(2));
      }
      _ => {
        let _ = child.kill();
        let _ = child.wait();
        return None;
      }
    }
  };
  let mut output = String::new();
  child.stdout.take()?.read_to_string(&mut output).ok()?;
  status.success().then_some(output)
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
      clipboard: None,
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
        extended_keys: true,
        focus_events: true,
        origin: (0, 0),
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

  fn tmux_caps(version: Vec<u32>, client: &str, focus_events: bool) -> Caps {
    use crate::caps::Tmux;
    Caps {
      host: Host::Tmux(Some(Tmux {
        version,
        client: Some(client.into()),
        cell: None,
        passthrough: None,
        extended_keys: true,
        focus_events,
        origin: (0, 0),
      })),
      version: Some("tmux 3.8".into()),
      keyboard: None,
      sync: Support::Answered,
      pixels: Support::Missing,
      resize: Support::Missing,
      pointer: Support::Missing,
      drop: Support::Missing,
      clipboard: None,
      cell: None,
      attributes: Some(vec![1, 2, 4]),
    }
  }

  /// tmux 3.7 sends passthrough ahead of text it holds in a frame, and
  /// 3.7b and 3.7c drop the next image after one ends.
  #[test]
  fn no_frames_under_tmux_before_3_8() {
    let tmux = |version| tmux_caps(version, "kitty(0.49.1)", true);
    let direct =
      Caps { host: Host::Direct(crate::caps::Graphics::Ok), ..tmux(vec![]) };
    let cases = [
      (None, true),
      (Some(direct), true),
      (Some(tmux(vec![3, 8])), true),
      (Some(tmux(vec![3, 9])), true),
      (Some(tmux(vec![])), true),
      (Some(tmux(vec![3, 7])), false),
      (Some(tmux(vec![3, 3])), false),
    ];
    for (caps, want) in cases {
      assert_eq!(synced(caps.as_ref()), want, "{caps:?}");
    }
  }

  #[test]
  fn tmux_notices_say_what_lacks() {
    let set = || Some("Set tmux allow-passthrough all".to_owned());
    let off = "tmux focus-events off: images, shapes outlast window switches";
    let ctrl_i = "Ctrl-i is Tab under tmux in Kitty";
    let cases = [
      (
        tmux_caps(vec![3, 8], "kitty(0.49.1)", true),
        set(),
        vec![set().unwrap(), ctrl_i.into()],
      ),
      (
        tmux_caps(vec![3, 8], "kitty(0.49.1)", false),
        None,
        vec![off.into(), ctrl_i.into()],
      ),
      (tmux_caps(vec![3, 8], "WezTerm 20240203", true), None, vec![]),
    ];
    for (caps, passthrough, want) in cases {
      assert_eq!(tmux_notices(&caps, passthrough), want);
    }
  }

  #[test]
  fn shapes_write_only_changes() {
    use Shape::{Default, Pointer, Text};
    #[derive(Clone, Copy, Debug)]
    enum Step {
      Push,
      Point(Shape),
      Forget,
      Pop,
    }
    let steps = [
      Step::Point(Text),
      Step::Push,
      Step::Push,
      Step::Point(Default),
      Step::Point(Text),
      Step::Point(Text),
      Step::Forget,
      Step::Point(Text),
      Step::Point(Pointer),
      Step::Pop,
      Step::Point(Text),
      Step::Pop,
    ];
    let run = |support, tmux| {
      let mut shapes = Shapes { owner: None, shown: None };
      let mut out = Vec::new();
      for step in steps {
        match step {
          Step::Push => shapes.push(support, &mut out, tmux),
          Step::Point(shape) => shapes.point(shape, &mut out, tmux),
          Step::Forget => {
            shapes.forget();
            Ok(())
          }
          Step::Pop => shapes.pop(&mut out, tmux),
        }
        .unwrap();
      }
      String::from_utf8(out).unwrap()
    };
    let osc = |bodies: &[&str], tmux| {
      let mut out = Vec::new();
      for body in bodies {
        osc22(&mut out, body, tmux).unwrap();
      }
      String::from_utf8(out).unwrap()
    };
    let kitty = [">default", "text", "text", "pointer", "<"];
    // Ghostty starts and ends at `text`.
    let ghostty = ["default", "text", "text", "pointer", "text"];
    // (support, tmux) -> written
    let cases = [
      (Support::Answered, false, osc(&kitty, false)),
      (Support::Named, false, osc(&ghostty, false)),
      (Support::Missing, false, String::new()),
      (Support::Answered, true, osc(&kitty, true)),
    ];
    for (support, tmux, want) in cases {
      assert_eq!(run(support, tmux), want, "{support:?} {tmux}");
    }
    let mut wrapped = Vec::new();
    osc22(&mut wrapped, "<", true).unwrap();
    assert_eq!(wrapped, b"\x1bPtmux;\x1b\x1b]22;<\x1b\x1b\\\x1b\\");
  }

  #[test]
  fn tmux_view_words() {
    let cases = [
      ("hidden", Some(View::Hidden)),
      ("inactive", Some(View::Inactive)),
      ("focused", Some(View::Active { focused: true })),
      ("away", Some(View::Active { focused: false })),
      ("", None),
      ("1", None),
    ];
    for (word, want) in cases {
      assert_eq!(view(word), want, "{word}");
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
