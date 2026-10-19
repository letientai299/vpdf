use crate::Result;
use std::path::PathBuf;
use std::process::Command;
use vpdf_terminal::terminal;

/// Edits `text` in `$VISUAL` or `$EDITOR`, like zsh's
/// `edit-command-line`; lines join with spaces.
pub fn edit(text: &str) -> Result<String> {
  let editor = ["VISUAL", "EDITOR"]
    .iter()
    .find_map(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty()))
    .unwrap_or_else(|| "vi".into());
  let mut words = editor.split_whitespace();
  let program = words.next().unwrap_or("vi").to_owned();
  let path = scratch()?;
  std::fs::write(&path, format!("{text}\n"))?;
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

/// A fresh file only this user can read.
fn scratch() -> Result<PathBuf> {
  use std::os::unix::fs::OpenOptionsExt;
  let directory = std::env::temp_dir();
  for attempt in 0..100 {
    let path =
      directory.join(format!("vpdf-line-{}-{attempt}.txt", std::process::id()));
    let created = std::fs::OpenOptions::new()
      .write(true)
      .create_new(true)
      .mode(0o600)
      .open(&path);
    if created.is_ok() {
      return Ok(path);
    }
  }
  anyhow::bail!("No free scratch file in {}", directory.display())
}
