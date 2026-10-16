use crate::Result;
use crate::terminal;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Directories that rarely hold documents and are slow to walk.
const SKIP: &[&str] = &["node_modules", "target", "__pycache__"];

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
  let preview = format!("'{exe}' text {{}} --page 1");
  let mut child = Command::new("fzf")
    .args(["--prompt", "PDF> ", "--preview", &preview])
    .args(["--preview-window", "right,55%,wrap"])
    .current_dir(root)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .spawn()?;
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

/// Streams relative PDF paths; stops when `fzf` closes its input.
fn walk(root: &Path, relative: &Path, out: &mut impl Write) -> Result<()> {
  let mut entries: Vec<_> = std::fs::read_dir(root.join(relative))?
    .filter_map(std::result::Result::ok)
    .collect();
  entries.sort_by_key(std::fs::DirEntry::file_name);
  for entry in entries {
    let name = entry.file_name();
    let text = name.to_string_lossy();
    if text.starts_with('.') || SKIP.contains(&text.as_ref()) {
      continue;
    }
    let path = relative.join(&name);
    let Ok(kind) = entry.file_type() else { continue };
    if kind.is_dir() {
      let _ = walk(root, &path, out);
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
  fn walk_lists_visible_pdfs_only() -> Result<()> {
    let root = tempfile::tempdir()?;
    for path in ["a.pdf", "b.txt", "docs/c.PDF", ".git/d.pdf", "target/e.pdf"] {
      let path = root.path().join(path);
      std::fs::create_dir_all(path.parent().unwrap_or(root.path()))?;
      std::fs::write(path, "")?;
    }
    let mut out = Vec::new();
    walk(root.path(), Path::new(""), &mut out)?;
    assert_eq!(String::from_utf8(out)?, "a.pdf\ndocs/c.PDF\n");
    Ok(())
  }
}
