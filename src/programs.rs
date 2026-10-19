//! Programs that take the terminal, and the private files they read.

use crate::Result;
use anyhow::Context;
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use vpdf_terminal::terminal;

/// Creates `vpdf-{stem}-{pid}-{n}.{extension}`.
pub fn scratch_file(stem: &str, extension: &str) -> Result<(PathBuf, File)> {
  static NEXT: AtomicU64 = AtomicU64::new(0);
  let directory = std::env::temp_dir();
  for _ in 0..100 {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let name = format!("vpdf-{stem}-{}-{n}.{extension}", std::process::id());
    let path = directory.join(name);
    let created =
      OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path);
    match created {
      Ok(file) => return Ok((path, file)),
      Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
      Err(error) => return Err(error.into()),
    }
  }
  anyhow::bail!("No free scratch file in {}", directory.display())
}

/// Edits `text` in `$VISUAL` or `$EDITOR`, like zsh's
/// `edit-command-line`; lines join with spaces.
pub fn edit(text: &str) -> Result<String> {
  let editor = ["VISUAL", "EDITOR"]
    .iter()
    .find_map(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty()))
    .unwrap_or_else(|| "vi".into());
  let mut words = editor.split_whitespace();
  let program = words.next().unwrap_or("vi").to_owned();
  let (path, mut file) = scratch_file("line", "txt")?;
  let written = writeln!(file, "{text}");
  drop(file);
  if let Err(error) = written {
    let _ = std::fs::remove_file(&path);
    return Err(error.into());
  }
  terminal::suspend()?;
  let status = Command::new(&program).args(words).arg(&path).status();
  terminal::resume()?;
  let edited = std::fs::read_to_string(&path);
  let _ = std::fs::remove_file(&path);
  anyhow::ensure!(status?.success(), "{program} exited with an error");
  Ok(
    edited?
      .lines()
      .map(str::trim)
      .filter(|l| !l.is_empty())
      .collect::<Vec<_>>()
      .join(" "),
  )
}

/// Version-control metadata and dependency trees, which never hold the
/// user's documents. Hidden and VCS-ignored folders such as `.ai` stay in.
const SKIP: &[&str] = &[".git", ".hg", ".svn", ".jj", "node_modules"];

/// Runs `fzf` over PDFs below `root`; `None` when cancelled.
pub fn pick_pdf(root: &Path) -> Result<Option<PathBuf>> {
  terminal::suspend()?;
  let result = run_fzf(root);
  terminal::resume()?;
  result
}

fn run_fzf(root: &Path) -> Result<Option<PathBuf>> {
  let exe = std::env::current_exe()?;
  let preview = preview(&exe, crate::inline::Graphics::probed());
  let mut child = Command::new("fzf")
    .args(["--prompt", "PDF> ", "--preview", &preview])
    .args(["--preview-window", "right,55%"])
    .current_dir(root)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .spawn()
    .context("Cannot run fzf")?;
  let input = child.stdin.take();
  let start = root.to_owned();
  std::thread::spawn(move || {
    if let Some(input) = input {
      let mut input = BufWriter::new(input);
      let _ = walk(&start, Path::new(""), &mut input);
      let _ = input.flush();
    }
  });
  let output = child.wait_with_output()?;
  let choice = String::from_utf8_lossy(&output.stdout);
  let choice = choice.trim_end_matches(['\n', '\r']);
  Ok((output.status.success() && !choice.is_empty()).then(|| root.join(choice)))
}

/// The fzf preview command; the variable stays scoped to it, so fzf
/// binds that run vpdf open the reader.
fn preview(exe: &Path, graphics: crate::inline::Graphics) -> String {
  let exe = exe.to_string_lossy().replace('\'', r"'\''");
  let name = graphics.name();
  format!("env {}={name} '{exe}' -- {{}}", crate::inline::VAR)
}

/// Streams relative PDF paths, skipping folders it cannot read; stops
/// when `fzf` closes its input.
fn walk(root: &Path, relative: &Path, out: &mut impl Write) -> io::Result<()> {
  let Ok(entries) = std::fs::read_dir(root.join(relative)) else {
    return Ok(());
  };
  let mut entries: Vec<_> =
    entries.filter_map(std::result::Result::ok).collect();
  entries.sort_by_key(std::fs::DirEntry::file_name);
  for entry in entries {
    let name = entry.file_name();
    let text = name.to_string_lossy();
    if SKIP.contains(&text.as_ref()) {
      continue;
    }
    let path = relative.join(&name);
    let Ok(kind) = entry.file_type() else { continue };
    if kind.is_dir() {
      walk(root, &path, out)?;
    } else if crate::cmdline::is_pdf(Path::new(&name)) {
      writeln!(out, "{}", path.display())?;
    }
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn preview_scopes_its_variable_and_ends_options() {
    let command =
      preview(Path::new("/a/it's/vpdf"), crate::inline::Graphics::Kitty);
    assert_eq!(command, r"env VPDF_PREVIEW=kitty '/a/it'\''s/vpdf' -- {}");
  }

  #[test]
  fn walk_includes_hidden_folders_but_not_vcs() -> Result<()> {
    let root = tempfile::tempdir()?;
    for path in ["a.pdf", "b.txt", "docs/c.PDF", ".git/d.pdf", ".ai/e.pdf"] {
      let path = root.path().join(path);
      std::fs::create_dir_all(path.parent().unwrap_or(root.path()))?;
      std::fs::write(path, "")?;
    }
    let mut out = Vec::new();
    walk(root.path(), Path::new(""), &mut out)?;
    assert_eq!(String::from_utf8(out)?, ".ai/e.pdf\na.pdf\ndocs/c.PDF\n");
    Ok(())
  }

  /// `fzf` closed its input.
  struct Closed;

  impl Write for Closed {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
      Err(io::ErrorKind::BrokenPipe.into())
    }
    fn flush(&mut self) -> io::Result<()> {
      Ok(())
    }
  }

  #[test]
  fn walk_stops_when_fzf_closes() -> Result<()> {
    let root = tempfile::tempdir()?;
    std::fs::create_dir(root.path().join("docs"))?;
    std::fs::write(root.path().join("docs/a.pdf"), "")?;
    assert!(walk(root.path(), Path::new(""), &mut Closed).is_err());
    Ok(())
  }
}
