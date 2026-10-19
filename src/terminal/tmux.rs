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
