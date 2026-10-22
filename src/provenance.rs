use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub struct Source {
  pub revision: String,
  pub dirty: bool,
  pub patch: String,
}

impl Source {
  pub fn read(directory: &Path) -> Option<Self> {
    Self::read_using(directory, "git")
  }

  pub(crate) fn read_using(directory: &Path, executable: &str) -> Option<Self> {
    let root = command(executable, directory)
      .args(["rev-parse", "--show-toplevel"])
      .output()
      .ok()?;
    if !root.status.success() {
      return None;
    }
    let root =
      std::path::PathBuf::from(String::from_utf8(root.stdout).ok()?.trim_end());
    let directory = root.as_path();
    let git = |args: &[&str]| {
      let out = command(executable, directory).args(args).output().ok()?;
      out.status.success().then_some(out.stdout)
    };
    let revision =
      String::from_utf8(git(&["rev-parse", "HEAD"])?).ok()?.trim().to_owned();
    let status = git(&["status", "--porcelain", "--untracked-files=all"])?;
    let mut bytes = git(&["diff", "--binary", "HEAD", "--"])?;
    let untracked = git(&["ls-files", "--others", "--exclude-standard", "-z"])?;
    for path in untracked.split(|byte| *byte == 0).filter(|p| !p.is_empty()) {
      let name = std::str::from_utf8(path).ok()?;
      bytes.extend_from_slice(path);
      bytes.push(0);
      let contents = std::fs::read(directory.join(name)).ok()?;
      bytes.extend_from_slice(&(contents.len() as u64).to_le_bytes());
      bytes.extend_from_slice(&contents);
    }
    let mut child = command(executable, directory)
      .args(["hash-object", "--stdin"])
      .stdin(Stdio::piped())
      .stdout(Stdio::piped())
      .stderr(Stdio::null())
      .spawn()
      .ok()?;
    child.stdin.take()?.write_all(&bytes).ok()?;
    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
      return None;
    }
    let patch = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    Some(Self { revision, dirty: !status.is_empty(), patch })
  }
}

pub(crate) fn command(executable: &str, directory: &Path) -> Command {
  let mut command = Command::new(executable);
  command.current_dir(directory);
  for name in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR"] {
    command.env_remove(name);
  }
  command
}
