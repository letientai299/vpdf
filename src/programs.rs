//! Programs that take the terminal, and the private files they read.

use crate::Result;
use anyhow::Context;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::task::Poll;
use tempfile::NamedTempFile;
use vpdf_terminal::terminal;

pub fn scratch_file(stem: &str, extension: &str) -> Result<NamedTempFile> {
  Ok(
    tempfile::Builder::new()
      .prefix(&format!("vpdf-{stem}-"))
      .suffix(&format!(".{extension}"))
      .tempfile()?,
  )
}

/// Edits `text` in `$VISUAL` or `$EDITOR`, like zsh's
/// `edit-command-line`; lines join with spaces.
pub fn edit(text: &str, terminal: bool) -> Result<String> {
  anyhow::ensure!(terminal, "External editing requires a terminal");
  let editor = ["VISUAL", "EDITOR"]
    .iter()
    .find_map(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty()))
    .unwrap_or_else(|| "vi".into());
  let mut words = editor.split_whitespace();
  let program = words.next().unwrap_or("vi").to_owned();
  let mut command = Command::new(&program);
  command.args(words);
  edit_file(text, scratch_file("line", "txt")?, |path| {
    terminal::suspend()?;
    let status = command.arg(path).status();
    terminal::resume()?;
    anyhow::ensure!(status?.success(), "{program} exited with an error");
    Ok(())
  })
}

fn edit_file(
  text: &str,
  mut file: NamedTempFile,
  run: impl FnOnce(&Path) -> Result<()>,
) -> Result<String> {
  writeln!(file, "{text}")?;
  let path = file.into_temp_path();
  run(&path)?;
  Ok(
    std::fs::read_to_string(&path)?
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

/// A macOS dialog process; dropping it closes the dialog.
pub(crate) struct Dialog {
  pub(crate) child: std::process::Child,
}

impl Dialog {
  /// Runs `osascript` with `args`, then `path`.
  pub(crate) fn osascript(args: &[&str], path: &Path) -> Result<Self> {
    Self::spawn(Command::new("/usr/bin/osascript").args(args).arg(path))
  }

  pub(crate) fn spawn(command: &mut Command) -> Result<Self> {
    let child = command
      .stdin(Stdio::null())
      .stdout(Stdio::piped())
      .stderr(Stdio::null())
      .spawn()?;
    Ok(Self { child })
  }

  /// Its output once it exits; `failed` when it fails.
  pub(crate) fn poll(&mut self, failed: &str) -> Result<Poll<String>> {
    use std::io::Read;
    let Some(status) = self.child.try_wait()? else { return Ok(Poll::Pending) };
    anyhow::ensure!(status.success(), "{failed}");
    let mut output = String::new();
    if let Some(stdout) = self.child.stdout.take() {
      // Paths fit; runaway output stops here.
      stdout.take(1 << 16).read_to_string(&mut output)?;
    }
    Ok(Poll::Ready(output))
  }

  /// A finished dialog that ran `program` with `args`.
  #[cfg(test)]
  pub(crate) fn exited(program: &str, args: &[&str]) -> Result<Self> {
    let mut dialog = Self::spawn(Command::new(program).args(args))?;
    dialog.child.wait()?;
    Ok(dialog)
  }
}

impl Drop for Dialog {
  fn drop(&mut self) {
    let _ = self.child.kill();
    let _ = self.child.wait();
  }
}

pub(crate) struct Picker(Dialog);

impl Picker {
  pub fn start() -> Result<Self> {
    anyhow::ensure!(cfg!(target_os = "macos"), "Native picker requires macOS");
    let script = concat!(
      "on run argv\ntry\n",
      "set root to (POSIX file (item 1 of argv)) as alias\n",
      "POSIX path of (choose file of type {\"com.adobe.pdf\"} ",
      "default location root)\n",
      "on error number -128\nreturn \"\"\nend try\nend run"
    );
    Dialog::osascript(&["-e", script], &std::env::current_dir()?).map(Self)
  }

  pub fn poll(&mut self) -> Result<Poll<Option<PathBuf>>> {
    Ok(self.0.poll("Native picker failed")?.map(|path| {
      let path = path.strip_suffix('\n').unwrap_or(&path);
      (!path.is_empty()).then(|| PathBuf::from(path))
    }))
  }
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
  fn picker_output_status() -> Result<()> {
    for path in ["", "/tmp/a b\n.pdf"] {
      let mut picker =
        Picker(Dialog::exited("/usr/bin/printf", &["%s\n", path])?);
      let expected = (!path.is_empty()).then(|| PathBuf::from(path));
      assert_eq!(picker.poll()?, Poll::Ready(expected));
    }
    let mut picker = Picker(Dialog::exited("/usr/bin/false", &[])?);
    assert!(picker.poll().is_err());
    Ok(())
  }

  #[test]
  fn native_editor_requires_terminal() {
    let error = edit("private prompt", false).unwrap_err();
    assert_eq!(error.to_string(), "External editing requires a terminal");
  }

  #[test]
  fn editor_replacement_cleanup() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for fails in [false, true] {
      let file = scratch_file("line", "txt")?;
      let path = file.path().to_owned();
      assert_eq!(path.extension().unwrap(), "txt");
      assert_eq!(path.metadata()?.permissions().mode() & 0o777, 0o600);
      let result = edit_file("original", file, |path| {
        assert_eq!(std::fs::read_to_string(path)?, "original\n");
        let mut replacement = NamedTempFile::new_in(path.parent().unwrap())?;
        replacement.write_all(b"  edited\n\n  second line \n")?;
        replacement.persist(path)?;
        anyhow::ensure!(!fails, "Editor failed");
        Ok(())
      });
      if fails {
        assert!(result.is_err());
      } else {
        assert_eq!(result?, "edited second line");
      }
      assert!(!path.exists());
    }
    let file = scratch_file("line", "txt")?;
    let path = file.path().to_owned();
    let read_only = std::fs::File::open(&path)?;
    let file = NamedTempFile::from_parts(read_only, file.into_temp_path());
    assert!(edit_file("original", file, |_| panic!("write failed")).is_err());
    assert!(!path.exists());
    Ok(())
  }

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
