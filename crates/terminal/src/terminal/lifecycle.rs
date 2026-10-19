use super::{
  AtomicBool, Caps, Host, Keyboard, Mutex, Once, Ordering, PROVOKED,
  PoisonError, Result, Size, Support, Terminal, Write, cursor, event, execute,
  frame_end, io, note, shapes, terminal, tmux,
};

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

/// Cleared by `:mouse off`, which leaves the pointer to the terminal.
static MOUSE: AtomicBool = AtomicBool::new(true);

/// Set while tmux sends the pane focus events, mode 1004, by which the
/// session hides placements tmux would leave over another window.
pub(super) static FOCUS: AtomicBool = AtomicBool::new(false);

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
  if mouse() {
    grab_mouse(&mut io::stdout(), caps)?;
  }
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

/// Whether the reader takes mouse reports.
pub fn mouse() -> bool {
  MOUSE.load(Ordering::Relaxed)
}

/// `:mouse`: takes the pointer, or leaves it to the terminal's own
/// selection; resumes after child programs keep the choice.
pub fn capture_mouse(on: bool) -> Result<()> {
  if MOUSE.swap(on, Ordering::Relaxed) == on || !ACTIVE.load(Ordering::SeqCst) {
    return Ok(());
  }
  let mut out = io::stdout();
  match (on, crate::caps::get()) {
    (true, Some(caps)) => grab_mouse(&mut out, caps)?,
    // Unprobed: resume asks first.
    (true, None) => {}
    (false, _) => free_mouse(&mut out)?,
  }
  out.flush()?;
  Ok(())
}

/// Asks for button, motion, and SGR reports, pixels where they are
/// screen pixels, and pushes a pointer shape.
pub(super) fn grab_mouse(out: &mut impl Write, caps: &Caps) -> Result<()> {
  crossterm::queue!(out, event::EnableMouseCapture)?;
  if caps.pixel_mouse() {
    // The parser first, so no pixel report reads as cells.
    crate::input::set_pixels(true);
    PIXELS.store(true, Ordering::Relaxed);
    out.write_all(b"\x1b[?1016h")?;
  }
  // Reports switch now, not at the first frame.
  out.flush()?;
  super::claim_pointer(out, caps.pointer, tmux())
}

/// Undoes `grab_mouse`; the parser keeps pixels for reports in flight.
pub(super) fn free_mouse(out: &mut impl Write) -> Result<()> {
  if PIXELS.swap(false, Ordering::Relaxed) {
    out.write_all(b"\x1b[?1016l")?;
  }
  shapes().pop(out, tmux())?;
  crossterm::queue!(out, event::DisableMouseCapture)?;
  Ok(())
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
pub(super) fn tmux_notices(
  caps: &Caps,
  passthrough: Option<String>,
) -> Vec<String> {
  let mut notices = Vec::from_iter(passthrough);
  if matches!(&caps.host, Host::Tmux(Some(tmux)) if !tmux.focus_events) {
    notices
      .push("tmux focus-events off: pointer cleanup waits for polling".into());
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
  free_mouse(&mut io::stdout())?;
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
pub(super) fn reports() -> bool {
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
