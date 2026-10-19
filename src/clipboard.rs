use crate::Result;
use crate::app::App;
use anyhow::{Context, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use vpdf_terminal::clipboard::osc52;

/// Unnamed and selection registers mirror the system clipboard.
pub fn synced(register: char) -> bool {
  matches!(register, '"' | '+' | '*')
}

impl App {
  pub fn clipboard(&mut self, out: &mut impl Write) -> Result<()> {
    let Some(yank) = self.reader.take_yank() else { return Ok(()) };
    if synced(yank.register) {
      copy(yank.text, out)?;
    }
    Ok(())
  }
}

/// Sends OSC 52 and, on a local session, the native tool.
pub fn copy(text: String, out: &mut impl Write) -> Result<()> {
  osc52(out, &STANDARD.encode(&text), vpdf_terminal::terminal::tmux())?;
  if let Some(tools) = native() {
    std::thread::spawn(move || {
      tools.iter().any(|(program, args)| pipe(program, args, &text))
    });
  }
  Ok(())
}

pub fn image_native(png: Vec<u8>) -> Result<()> {
  ensure!(!remote(), "Native clipboard unavailable over SSH");
  let tool = if cfg!(target_os = "macos") {
    ImageTool::Mac
  } else if env("WAYLAND_DISPLAY") {
    ImageTool::Wayland
  } else {
    ensure!(env("DISPLAY"), "Image clipboard unavailable");
    ImageTool::X11
  };
  let (command, file) = image_command(tool, &png)?;
  run_image(command, file.is_none().then_some(png), Duration::from_secs(10))
}

#[derive(Clone, Copy)]
enum ImageTool {
  Mac,
  Wayland,
  X11,
}

fn image_command(
  tool: ImageTool,
  png: &[u8],
) -> Result<(Command, Option<TempPng>)> {
  let mut command;
  let mut file = None;
  match tool {
    ImageTool::Mac => {
      let temp = TempPng::new(png)?;
      command = Command::new("osascript");
      command.args([
        "-e",
        concat!(
          "on run argv\n",
          "set pngPath to item 1 of argv\n",
          "set pngFile to POSIX file pngPath\n",
          "set imageData to read pngFile as «class PNGf»\n",
          "set the clipboard to imageData\n",
          "end run",
        ),
      ]);
      command.arg(&temp.0);
      file = Some(temp);
    }
    ImageTool::Wayland => {
      command = Command::new("wl-copy");
      command.args(["--type", "image/png"]);
    }
    ImageTool::X11 => {
      command = Command::new("xclip");
      command.args(["-selection", "clipboard", "-t", "image/png"]);
    }
  }
  Ok((command, file))
}

struct TempPng(PathBuf);

impl TempPng {
  fn new(png: &[u8]) -> Result<Self> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    for _ in 0..100 {
      let name = format!(
        "vpdf-clipboard-{}-{}.png",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
      );
      let path = std::env::temp_dir().join(name);
      match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
      {
        Ok(mut file) => {
          let temp = Self(path);
          file.write_all(png)?;
          return Ok(temp);
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
      }
    }
    anyhow::bail!("Clipboard temporary file unavailable")
  }
}

impl Drop for TempPng {
  fn drop(&mut self) {
    let _ = std::fs::remove_file(&self.0);
  }
}

fn run_image(
  mut command: Command,
  png: Option<Vec<u8>>,
  timeout: Duration,
) -> Result<()> {
  let mut child = command
    .stdin(if png.is_some() { Stdio::piped() } else { Stdio::null() })
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .context("Image clipboard tool unavailable")?;
  let input = child.stdin.take();
  let writer = std::thread::spawn(move || -> std::io::Result<()> {
    if let (Some(mut input), Some(png)) = (input, png) {
      input.write_all(&png)?;
    }
    Ok(())
  });
  let started = Instant::now();
  let status = loop {
    match child.try_wait() {
      Ok(Some(status)) => break status,
      Ok(None) if started.elapsed() < timeout => {
        std::thread::sleep(Duration::from_millis(10));
      }
      result => {
        let _ = child.kill();
        let _ = child.wait();
        let _ = writer.join();
        result?;
        anyhow::bail!("Image clipboard timed out");
      }
    }
  };
  writer.join().map_err(|_| anyhow::anyhow!("Clipboard writer failed"))??;
  ensure!(status.success(), "Image clipboard tool failed");
  Ok(())
}

type Tool = (&'static str, &'static [&'static str]);

fn env(name: &str) -> bool {
  std::env::var_os(name).is_some_and(|v| !v.is_empty())
}

pub(crate) fn remote() -> bool {
  env("SSH_CONNECTION") || env("SSH_TTY")
}

// Remote native tools would fill the remote host's clipboard.
fn native() -> Option<&'static [Tool]> {
  if remote() {
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
  let mut command = Command::new(program);
  command.args(args);
  run_image(command, Some(text.as_bytes().to_vec()), Duration::from_secs(10))
    .is_ok()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn default_register_reaches_clipboard() {
    assert!(synced('"') && synced('+') && synced('*'));
    assert!(!synced('a') && !synced('0'));
  }

  #[test]
  fn native_images_use_owned_tools() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir()?;
    let mock = temp.path().join("clipboard mock");
    std::fs::write(
      &mock,
      r#"#!/bin/sh
if [ "$MODE" = mac ]; then
    cat "$3" > "$CAPTURE"
else
    cat > "$CAPTURE"
fi
"#,
    )?;
    std::fs::set_permissions(&mock, std::fs::Permissions::from_mode(0o700))?;
    let png = b"\x89PNG\r\n\x1a\n\0\xfftest";
    for tool in [ImageTool::Mac, ImageTool::Wayland, ImageTool::X11] {
      let (command, file) = image_command(tool, png)?;
      let args: Vec<_> = command.get_args().collect();
      match tool {
        ImageTool::Mac => {
          assert_eq!(command.get_program(), "osascript");
          assert_eq!(args[0], "-e");
          assert!(args[1].to_string_lossy().contains("«class PNGf»"));
          let path = &file.as_ref().unwrap().0;
          assert_eq!(
            std::fs::metadata(path)?.permissions().mode() & 0o777,
            0o600
          );
          assert_eq!(std::fs::read(path)?, png);
        }
        ImageTool::Wayland => {
          assert_eq!(command.get_program(), "wl-copy");
          assert_eq!(args, ["--type", "image/png"]);
        }
        ImageTool::X11 => {
          assert_eq!(command.get_program(), "xclip");
          assert_eq!(args, ["-selection", "clipboard", "-t", "image/png"]);
        }
      }
      let capture = temp.path().join("copied.png");
      let mut command = Command::new(&mock);
      command
        .args(args)
        .env("MODE", if file.is_some() { "mac" } else { "stdin" })
        .env("CAPTURE", &capture);
      run_image(
        command,
        file.is_none().then(|| png.to_vec()),
        Duration::from_secs(2),
      )?;
      assert_eq!(std::fs::read(capture)?, png);
      let path = file.as_ref().map(|file| file.0.clone());
      drop(file);
      assert!(path.is_none_or(|path| !path.exists()));
    }
    let mut failed = Command::new("/bin/sh");
    failed.args(["-c", "exit 7"]);
    assert!(run_image(failed, None, Duration::from_secs(1)).is_err());
    let mut stalled = Command::new("/bin/sleep");
    stalled.arg("5");
    let error =
      run_image(stalled, None, Duration::from_millis(20)).unwrap_err();
    assert_eq!(error.to_string(), "Image clipboard timed out");
    assert!(
      run_image(
        Command::new(temp.path().join("missing")),
        None,
        Duration::from_secs(1)
      )
      .is_err()
    );
    Ok(())
  }
}
