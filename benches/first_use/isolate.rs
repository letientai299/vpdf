use std::io::{Read, Seek};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::NamedTempFile;
use vpdf::Result;

const OUTPUT_LIMIT: u64 = 65_536;

pub struct Runner {
  stdout: NamedTempFile,
  stderr: NamedTempFile,
  bytes: Vec<u8>,
}

impl Runner {
  pub fn new() -> Result<Self> {
    Ok(Self {
      stdout: NamedTempFile::new()?,
      stderr: NamedTempFile::new()?,
      bytes: Vec::new(),
    })
  }

  pub fn run(
    &mut self,
    command: &mut Command,
    timeout: Duration,
  ) -> Result<&[u8]> {
    for file in [&mut self.stdout, &mut self.stderr] {
      file.as_file_mut().set_len(0)?;
      file.rewind()?;
    }
    let started = Instant::now();
    let mut child = command
      .stdin(Stdio::null())
      .stdout(self.stdout.as_file().try_clone()?)
      .stderr(self.stderr.as_file().try_clone()?)
      .spawn()?;
    let status = loop {
      match child.try_wait() {
        Ok(Some(status)) => break status,
        Ok(None) if started.elapsed() < timeout => {
          std::thread::sleep(Duration::from_millis(1));
        }
        result => {
          let _ = child.kill();
          let _ = child.wait();
          let _ = result?;
          anyhow::bail!("Interaction child timed out");
        }
      }
    };
    self.bytes.clear();
    let file =
      if status.success() { &mut self.stdout } else { &mut self.stderr };
    file.rewind()?;
    file.take(OUTPUT_LIMIT + 1).read_to_end(&mut self.bytes)?;
    anyhow::ensure!(
      self.bytes.len() as u64 <= OUTPUT_LIMIT,
      "Child output exceeds limit"
    );
    anyhow::ensure!(
      status.success(),
      "Interaction child failed: {}",
      String::from_utf8_lossy(&self.bytes)
    );
    Ok(&self.bytes)
  }
}
