use super::{Duration, FOCUS, Instant, Ordering, Read};

/// Whether output passes through tmux, as the probe routes it.
pub fn tmux() -> bool {
  crate::caps::under_tmux()
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
pub(crate) const VIEW: &str = "#{?#{&&:#{window_active_clients},\
  #{||:#{pane_active},#{?window_zoomed_flag,0,1}}},\
  #{?pane_active,#{?#{m:*focused*,#{client_flags}},focused,away},inactive},\
  hidden}";

/// What tmux shows of vpdf's pane now; None outside tmux or when tmux
/// fails.
pub fn pane_view() -> Option<View> {
  view(tmux_display(VIEW)?.trim())
}

pub(crate) fn view(word: &str) -> Option<View> {
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

/// What the reader knows of tmux showing its pane. tmux sends focus
/// events only to a window's active pane, so an unfocused pane asks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Sight {
  /// Takes keys, or tmux is not asked: a focus event says what changes.
  #[default]
  Focused,
  /// Shown without keys; the idle tick asks tmux whether it still is.
  Unfocused,
  /// tmux shows something else over it, such as another window or a
  /// popup: frames wait, since tmux leaves placements over it.
  Hidden,
}

/// Why the reader asks tmux how it shows the pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cause {
  Start,
  FocusOut,
  Tick,
}

/// The sight after `cause`, given tmux's `view` of the pane. A focus-out
/// that leaves the pane active with a focused client is a popup or
/// menu. Without the OS focus the client tells nothing of a window
/// switch another client or a script makes, so the pane asks.
pub fn sight(now: Sight, cause: Cause, view: View) -> Sight {
  match (cause, view) {
    (_, View::Hidden) | (Cause::FocusOut, View::Active { focused: true }) => {
      Sight::Hidden
    }
    (_, View::Inactive)
    | (Cause::Start | Cause::FocusOut, View::Active { focused: false }) => {
      Sight::Unfocused
    }
    (Cause::Start, View::Active { focused: true }) => Sight::Focused,
    (Cause::Tick, View::Active { .. }) => now,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// tmux tells only its active pane about focus; the rest is asked.
  #[test]
  fn sight_follows_tmux() {
    use Cause::{FocusOut, Start, Tick};
    use Sight::{Focused, Hidden, Unfocused};
    let active = View::Active { focused: true };
    let away = View::Active { focused: false };
    // (sight, cause, view, want)
    let cases = [
      (Focused, Start, active, Focused),
      (Focused, Start, View::Inactive, Unfocused),
      (Focused, Start, View::Hidden, Hidden),
      (Focused, FocusOut, View::Inactive, Unfocused),
      (Focused, FocusOut, View::Hidden, Hidden),
      // A popup or menu over the active pane.
      (Focused, FocusOut, active, Hidden),
      // The client lost the OS focus.
      (Focused, FocusOut, away, Unfocused),
      (Focused, Start, away, Unfocused),
      (Unfocused, Tick, View::Hidden, Hidden),
      (Unfocused, Tick, View::Inactive, Unfocused),
      (Hidden, Tick, View::Inactive, Unfocused),
      (Hidden, Tick, View::Hidden, Hidden),
      // The popup is still open; focus-in ends it.
      (Hidden, Tick, active, Hidden),
      (Unfocused, Tick, away, Unfocused),
    ];
    for (now, cause, view, want) in cases {
      assert_eq!(sight(now, cause, view), want, "{now:?} {cause:?} {view:?}");
    }
  }
}
