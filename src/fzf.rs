use crate::Result;
use crate::terminal;
use anyhow::Context;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Version-control metadata and dependency trees, which never hold the
/// user's documents. Hidden and VCS-ignored folders such as `.ai` stay in.
const SKIP: &[&str] = &[".git", ".hg", ".svn", ".jj", "node_modules"];

/// Runs `fzf` over PDFs below `root`; `None` when cancelled.
pub fn pick(root: &Path) -> Result<Option<PathBuf>> {
  terminal::suspend()?;
  let result = run(root);
  terminal::resume()?;
  result
}

fn run(root: &Path) -> Result<Option<PathBuf>> {
  let exe = std::env::current_exe()?;
  let exe = exe.to_string_lossy().replace('\'', r"'\''");
  let preview = format!("'{exe}' preview {{}}");
  let mut child = Command::new("fzf")
    .args(["--prompt", "PDF> ", "--preview", &preview])
    .args(["--preview-window", "right,55%"])
    .env(crate::inline::VAR, crate::inline::Graphics::probed().name())
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
    } else if Path::new(&name)
      .extension()
      .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
    {
      writeln!(out, "{}", path.display())?;
    }
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

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
