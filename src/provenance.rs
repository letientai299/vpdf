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
    let root = Command::new(executable)
      .args(["rev-parse", "--show-toplevel"])
      .current_dir(directory)
      .output()
      .ok()?;
    if !root.status.success() {
      return None;
    }
    let root =
      std::path::PathBuf::from(String::from_utf8(root.stdout).ok()?.trim_end());
    let directory = root.as_path();
    let git = |args: &[&str]| {
      let out = Command::new(executable)
        .args(args)
        .current_dir(directory)
        .output()
        .ok()?;
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
    let mut child = Command::new(executable)
      .args(["hash-object", "--stdin"])
      .current_dir(directory)
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
