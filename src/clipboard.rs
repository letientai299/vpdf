use crate::Result;
use crate::app::App;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::io::Write;
use std::process::{Command, Stdio};

/// Unnamed and selection registers mirror the system clipboard.
pub fn synced(register: char) -> bool {
  matches!(register, '"' | '+' | '*')
}

impl App {
  pub fn clipboard(&mut self, out: &mut impl Write) -> Result<()> {
    let Some(yank) = self.reader.pending_yank.take() else { return Ok(()) };
    if synced(yank.register) {
      copy(yank.text, out)?;
    }
    Ok(())
  }
}

/// Sends OSC 52 and, on a local session, the native tool.
pub fn copy(text: String, out: &mut impl Write) -> Result<()> {
  let data = STANDARD.encode(&text);
  if std::env::var_os("TMUX").is_some() {
    write!(out, "\x1bPtmux;\x1b\x1b]52;c;{data}\x07\x1b\\")?;
  } else {
    write!(out, "\x1b]52;c;{data}\x07")?;
  }
  if let Some(tools) = native() {
    std::thread::spawn(move || {
      tools.iter().any(|(program, args)| pipe(program, args, &text))
    });
  }
  Ok(())
}

type Tool = (&'static str, &'static [&'static str]);

// Remote native tools would fill the remote host's clipboard.
fn native() -> Option<&'static [Tool]> {
  let env = |name| std::env::var_os(name).is_some_and(|v| !v.is_empty());
  if env("SSH_CONNECTION") || env("SSH_TTY") {
    return None;
  }
  if cfg!(target_os = "macos") {
    return Some(&[("pbcopy", &[])]);
  }
  if env("WAYLAND_DISPLAY") {
    return Some(&[("wl-copy", &[])]);
  }
  env("DISPLAY").then_some(&[
    ("xclip", &["-selection", "clipboard"]),
    ("xsel", &["--clipboard", "--input"]),
  ])
}

fn pipe(program: &str, args: &[&str], text: &str) -> bool {
  let Ok(mut child) = Command::new(program)
    .args(args)
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
  else {
    return false;
  };
  let written = child
    .stdin
    .take()
    .is_some_and(|mut input| input.write_all(text.as_bytes()).is_ok());
  child.wait().is_ok_and(|status| status.success()) && written
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn default_register_reaches_clipboard() {
    assert!(synced('"') && synced('+') && synced('*'));
    assert!(!synced('a') && !synced('0'));
  }
}
