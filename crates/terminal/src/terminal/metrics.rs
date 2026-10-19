use super::{
  AtomicBool, Caps, Cell, Context, Host, Mutex, Ordering, PoisonError, Result,
  Screen, Size, WindowSize, reports, terminal, tmux,
};

/// Set while the report that turning reports on provokes is due.
pub(super) static PROVOKED: AtomicBool = AtomicBool::new(false);

pub const NO_CELL: &str = "Terminal reports no cell size in pixels";

/// What tmux puts in a pane's pixel size when it does not know the
/// client's cell size ([tmux.h][h]).
///
/// [h]: https://github.com/tmux/tmux/blob/master/tmux.h
pub(super) const TMUX_FAKE: Cell = Cell { width: 16, height: 32 };

/// The latest cell size the terminal told in band, by a mode 2048
/// report or a `CSI 16 t` answer after a resize.
static TOLD: Mutex<Option<Cell>> = Mutex::new(None);

pub(super) fn told() -> Option<Cell> {
  *TOLD.lock().unwrap_or_else(PoisonError::into_inner)
}

fn tell(cell: Cell) {
  *TOLD.lock().unwrap_or_else(PoisonError::into_inner) = Some(cell);
}

pub(super) fn window(report: Size) -> WindowSize {
  WindowSize {
    rows: report.rows,
    columns: report.columns,
    width: report.width,
    height: report.height,
  }
}

/// The cell size `window`'s pixels imply; None without pixels.
pub(super) fn window_cell(window: &WindowSize) -> Option<Cell> {
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
pub(super) fn pick(
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
pub(super) fn note(report: Size) {
  if let Some(cell) = window_cell(&window(report)) {
    tell(cell);
  }
}

/// Whether a SIGWINCH should ask `CSI 16 t` again: only without
/// pixels, and not while a later resize waits in the queue.
pub(super) fn asks_again(window: Option<Cell>, queued: bool) -> bool {
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

pub(super) fn screen(window: &WindowSize) -> Result<Screen> {
  // One `tmux display` gives the pane's origin and the client's cell.
  let now = if tmux() { crate::caps::tmux_now() } else { None };
  screen_with(window, now.as_ref())
}

pub(crate) fn screen_with(
  window: &WindowSize,
  now: Option<&crate::caps::Tmux>,
) -> Result<Screen> {
  let cell = pick(
    window_cell(window),
    told(),
    crate::caps::get(),
    now.is_some() || tmux(),
    || now.and_then(|tmux| tmux.cell),
  )
  .context(NO_CELL)?;
  Ok(Screen {
    columns: window.columns.max(1),
    rows: window.rows.max(2),
    origin: now.map_or_default(|tmux| tmux.origin),
    cell_width: cell.width,
    cell_height: cell.height,
  })
}
