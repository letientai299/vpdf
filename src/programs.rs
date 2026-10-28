//! Programs that take the terminal, and the private files they read.

use crate::Result;
use anyhow::Context;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
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
    .map_or(Editor::Program("vi"), Editor::Shell);
  edit_file(text, scratch_file("line", "txt")?, |path| {
    handoff(|| editor.run(path))
  })
}

/// Lends the terminal to a child program, then restores the reader.
///
/// Interrupts reach the child, not Vpdf, while it runs; the terminal is
/// restored even when `run` fails.
pub fn handoff<T>(run: impl FnOnce() -> Result<T>) -> Result<T> {
  let default = interrupts()?;
  default.store(false, Ordering::SeqCst);
  let result = terminal::suspend().and_then(|()| run());
  let resumed = terminal::resume();
  default.store(true, Ordering::SeqCst);
  // A dead terminal outranks the child's own failure.
  resumed?;
  result
}

/// Whether SIGINT and SIGQUIT keep their default action in Vpdf.
///
/// A caught signal, unlike an ignored one, resets to default in an
/// exec'd child, so the child stays interruptible.
fn interrupts() -> io::Result<&'static AtomicBool> {
  use signal_hook::consts::{SIGINT, SIGQUIT};
  static DEFAULT: OnceLock<Arc<AtomicBool>> = OnceLock::new();
  if let Some(default) = DEFAULT.get() {
    return Ok(default);
  }
  let default = Arc::new(AtomicBool::new(true));
  for signal in [SIGINT, SIGQUIT] {
    signal_hook::flag::register_conditional_default(
      signal,
      Arc::clone(&default),
    )?;
  }
  Ok(DEFAULT.get_or_init(|| default))
}

/// The terminal editor for `:config edit`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Editor {
  /// `$EDITOR`, run by `sh` so it keeps its arguments and quoting.
  Shell(String),
  /// A fallback found on `PATH`.
  Program(&'static str),
}

impl Editor {
  pub fn from_env() -> Result<Self> {
    Self::select(std::env::var("EDITOR").ok(), crate::cmdline::on_path)
  }

  /// `$EDITOR`, else `nvim`, else `vim`; never `$VISUAL` or `vi`.
  fn select(
    editor: Option<String>,
    found: impl Fn(&str) -> bool,
  ) -> Result<Self> {
    if let Some(editor) = editor.filter(|editor| !editor.trim().is_empty()) {
      return Ok(Self::Shell(editor));
    }
    ["nvim", "vim"]
      .into_iter()
      .find(|program| found(program))
      .map(Self::Program)
      .ok_or_else(|| anyhow::anyhow!("Set $EDITOR or install nvim or vim"))
  }

  fn name(&self) -> &str {
    match self {
      Self::Shell(editor) => editor,
      Self::Program(program) => program,
    }
  }

  fn command(&self, path: &Path) -> Command {
    let mut command = match self {
      Self::Shell(editor) => {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(format!("{editor} \"$@\"")).arg("vpdf-editor");
        command
      }
      Self::Program(program) => Command::new(program),
    };
    command.arg(path);
    command
  }

  fn run(&self, path: &Path) -> Result<()> {
    let status = self.command(path).status();
    let name = self.name();
    let status = status.with_context(|| format!("Cannot run {name}"))?;
    match status.code() {
      Some(0) => Ok(()),
      Some(127) if matches!(self, Self::Shell(_)) => {
        anyhow::bail!("Editor not found: {name}")
      }
      Some(code) => anyhow::bail!("{name} exited with status {code}"),
      None => anyhow::bail!("{name} was interrupted"),
    }
  }
}

/// Edits the config file in this terminal pane, then resumes Vpdf.
pub fn edit_config(path: &Path, editor: &Editor) -> Result<()> {
  handoff(|| editor.run(path))
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
  anyhow::ensure!(
    crate::cmdline::on_path("fzf"),
    "Install fzf and add it to PATH"
  );
  handoff(|| run_fzf(root))
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

pub(crate) struct Picker {
  dialog: Dialog,
  pub(crate) tab: bool,
}

impl Picker {
  pub fn start(root: &Path, tab: bool) -> Result<Self> {
    anyhow::ensure!(cfg!(target_os = "macos"), "Native picker requires macOS");
    let script = concat!(
      "on run argv\ntry\n",
      "set root to (POSIX file (item 1 of argv)) as alias\n",
      "POSIX path of (choose file of type {\"com.adobe.pdf\"} ",
      "default location root)\n",
      "on error number -128\nreturn \"\"\nend try\nend run"
    );
    let dialog =
      Dialog::osascript(&["-e", script], &std::env::current_dir()?.join(root))?;
    Ok(Self { dialog, tab })
  }

  #[cfg(test)]
  pub(crate) fn exited(tab: bool, path: Option<&str>) -> Result<Self> {
    let dialog = match path {
      Some(path) => Dialog::exited("/usr/bin/printf", &["%s\n", path])?,
      None => Dialog::exited("/usr/bin/false", &[])?,
    };
    Ok(Self { dialog, tab })
  }

  pub fn poll(&mut self) -> Result<Poll<Option<PathBuf>>> {
    Ok(self.dialog.poll("Native picker failed")?.map(|path| {
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
    .context("Cannot run fzf; install fzf on PATH")?;
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
  anyhow::ensure!(
    output.status.success() || matches!(output.status.code(), Some(1 | 130)),
    "fzf failed: {}",
    output.status
  );
  let choice = String::from_utf8_lossy(&output.stdout);
  let choice = choice.trim_end_matches(['\n', '\r']);
  Ok((output.status.success() && !choice.is_empty()).then(|| root.join(choice)))
}

/// The fzf preview command; the variable stays scoped to it, so fzf
/// binds that run vpdf open the reader.
fn preview(exe: &Path, graphics: crate::inline::Graphics) -> String {
  let exe = exe.to_string_lossy().replace('\'', r"'\''");
  let name = graphics.name();
  format!(
    "env {}={name} {}=1 '{exe}' -- {{}}",
    crate::inline::VAR,
    crate::inline::PATH_VAR
  )
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
      let mut picker = Picker {
        dialog: Dialog::exited("/usr/bin/printf", &["%s\n", path])?,
        tab: false,
      };
      let expected = (!path.is_empty()).then(|| PathBuf::from(path));
      assert_eq!(picker.poll()?, Poll::Ready(expected));
    }
    let mut picker =
      Picker { dialog: Dialog::exited("/usr/bin/false", &[])?, tab: false };
    assert!(picker.poll().is_err());
    Ok(())
  }

  #[test]
  fn config_editor_selection() {
    let found = |names: &'static [&'static str]| {
      move |program: &str| names.contains(&program)
    };
    for (editor, installed, expected) in [
      (Some("hx -c x"), &[][..], Some(Editor::Shell("hx -c x".into()))),
      (Some(" "), &["vim"][..], Some(Editor::Program("vim"))),
      (None, &["nvim", "vim"][..], Some(Editor::Program("nvim"))),
      (None, &["vim", "vi"][..], Some(Editor::Program("vim"))),
      (None, &["vi"][..], None),
    ] {
      let selected = Editor::select(editor.map(Into::into), found(installed));
      assert_eq!(selected.ok(), expected, "{editor:?} {installed:?}");
    }
  }

  #[test]
  fn config_editor_runs_and_reports() -> Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("new config.toml");
    std::fs::write(&source, "[reading]\nzoom = 2.0\n")?;
    let path = root.path().join("my config.toml");
    std::fs::write(&path, "")?;
    let copy = format!("cp '{}'", source.display());
    Editor::Shell(copy).run(&path)?;
    assert_eq!(std::fs::read(&path)?, std::fs::read(&source)?);
    for (editor, message) in [
      ("false", "false exited with status 1"),
      ("vpdf-missing-editor", "Editor not found: vpdf-missing-editor"),
      ("kill -TERM $$; :", "kill -TERM $$; : was interrupted"),
    ] {
      let error = Editor::Shell(editor.into()).run(&path).unwrap_err();
      assert_eq!(error.to_string(), message);
    }
    let error = Editor::Program("vpdf-missing-editor").run(&path).unwrap_err();
    assert_eq!(error.to_string(), "Cannot run vpdf-missing-editor");
    assert_eq!(std::fs::read(&path)?, std::fs::read(&source)?);
    Ok(())
  }

  #[test]
  fn children_take_interrupts() -> Result<()> {
    use signal_hook::consts::SIGINT;
    let default = interrupts()?;
    default.store(false, Ordering::SeqCst);
    signal_hook::low_level::raise(SIGINT)?;
    let path = NamedTempFile::new()?;
    let error = Editor::Shell("kill -INT $$; :".into()).run(path.path());
    default.store(true, Ordering::SeqCst);
    assert_eq!(
      error.unwrap_err().to_string(),
      "kill -INT $$; : was interrupted"
    );
    assert!(std::ptr::eq(default, interrupts()?));
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
    assert_eq!(
      command,
      r"env VPDF_PREVIEW=kitty VPDF_PREVIEW_PATH=1 '/a/it'\''s/vpdf' -- {}"
    );
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
