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
  osc52(out, &STANDARD.encode(&text), crate::terminal::tmux())?;
  if let Some(tools) = native() {
    std::thread::spawn(move || {
      tools.iter().any(|(program, args)| pipe(program, args, &text))
    });
  }
  Ok(())
}

/// OSC 52 with base64 `data`. Under tmux both plainly, which tmux takes
/// with `set-clipboard on`, and wrapped, which passes through to the
/// terminal with `allow-passthrough` ([tmux input.c][tmux]).
///
/// [tmux]: https://github.com/tmux/tmux/blob/3.8/input.c#L3379-L3380
fn osc52(out: &mut impl Write, data: &str, tmux: bool) -> Result<()> {
  write!(out, "\x1b]52;c;{data}\x07")?;
  if tmux {
    write!(out, "\x1bPtmux;\x1b\x1b]52;c;{data}\x07\x1b\\")?;
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

pub fn image_remote(
  out: &mut impl Write,
  png: &[u8],
  tmux: bool,
) -> Result<()> {
  let packet = |out: &mut _, body: &str| {
    let bytes = format!("\x1b]5522;{body}\x1b\\");
    crate::terminal::passthrough(out, bytes.as_bytes(), tmux)
  };
  packet(out, "type=write")?;
  for chunk in png.chunks(4095) {
    packet(
      out,
      &format!("type=wdata:mime=aW1hZ2UvcG5n;{}", STANDARD.encode(chunk)),
    )?;
  }
  packet(out, "type=wdata")
}

pub fn image_status(body: &str) -> Option<Result<()>> {
  let fields: Vec<_> = body.split(':').collect();
  if !fields.contains(&"type=write") {
    return None;
  }
  let status = fields.iter().find_map(|field| field.strip_prefix("status="))?;
  Some(if status == "DONE" {
    Ok(())
  } else {
    Err(anyhow::anyhow!("Image clipboard failed: {status}"))
  })
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
  fn tmux_gets_plain_and_wrapped_osc52() -> Result<()> {
    let plain: &[u8] = b"\x1b]52;c;aGk=\x07";
    let wrapped: &[u8] = b"\x1bPtmux;\x1b\x1b]52;c;aGk=\x07\x1b\\";
    for (tmux, want) in
      [(false, plain.to_vec()), (true, [plain, wrapped].concat())]
    {
      let mut out = Vec::new();
      osc52(&mut out, "aGk=", tmux)?;
      assert_eq!(out, want, "tmux {tmux}");
    }
    Ok(())
  }
  #[test]
  fn image_packets_roundtrip() -> Result<()> {
    use crate::decode::{Event, Parser, Reply};
    use std::collections::VecDeque;
    for size in [1, 4095, 4096, 8191] {
      let png: Vec<u8> = (0..size).map(|n| n as u8).collect();
      let mut out = Vec::new();
      image_remote(&mut out, &png, false)?;
      let mut events = VecDeque::new();
      Parser::default().feed(&out, &mut events);
      assert_eq!(
        events.pop_front(),
        Some(Event::Reply(Reply::Osc {
          code: 5522,
          body: "type=write".into()
        }))
      );
      assert_eq!(
        events.pop_back(),
        Some(Event::Reply(Reply::Osc {
          code: 5522,
          body: "type=wdata".into()
        }))
      );
      let mut decoded = Vec::new();
      for event in events {
        let Event::Reply(Reply::Osc { code: 5522, body }) = event else {
          panic!("Unexpected packet")
        };
        let payload =
          body.strip_prefix("type=wdata:mime=aW1hZ2UvcG5n;").unwrap();
        let chunk = STANDARD.decode(payload)?;
        assert!(chunk.len() <= 4096);
        decoded.extend(chunk);
      }
      assert_eq!(decoded, png);
    }
    let mut wrapped = Vec::new();
    image_remote(&mut wrapped, b"hi", true)?;
    let expected: [&[u8]; 3] = [
      b"\x1bPtmux;\x1b\x1b]5522;type=write\x1b\x1b\\\x1b\\",
      b"\x1bPtmux;\x1b\x1b]5522;type=wdata:mime=aW1hZ2UvcG5n;\
        aGk=\x1b\x1b\\\x1b\\",
      b"\x1bPtmux;\x1b\x1b]5522;type=wdata\x1b\x1b\\\x1b\\",
    ];
    assert_eq!(wrapped, expected.concat());
    assert!(image_status("type=write:status=DONE").unwrap().is_ok());
    assert!(image_status("type=write:status=EPERM").unwrap().is_err());
    assert!(image_status("type=read:status=DONE").is_none());
    Ok(())
  }

  #[test]
  fn native_images_use_owned_tools() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir()?;
    let mock = temp.path().join("clipboard mock");
    std::fs::write(
      &mock,
      r"#!/usr/bin/python3
import os, pathlib, sys
if os.environ['MODE'] == 'mac':
    content = pathlib.Path(sys.argv[-1]).read_bytes()
else:
    content = sys.stdin.buffer.read()
pathlib.Path(os.environ['CAPTURE']).write_bytes(content)
",
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
