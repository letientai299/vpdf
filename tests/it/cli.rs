use std::process::Command;
use vpdf::Result;

const HEADLINE: &str = concat!(
  "Vpdf: a Vim-inspired terminal PDF reader powered by MuPDF ",
  "and the Kitty graphics protocol."
);
const REPOSITORY: &str = "https://github.com/letientai299/vpdf";

#[test]
fn help_and_version_name_the_project() -> Result<()> {
  for flag in ["--help", "--version"] {
    let output = Command::new(env!("CARGO_BIN_EXE_vpdf")).arg(flag).output()?;
    let stdout = String::from_utf8(output.stdout)?;
    assert!(output.status.success(), "{flag}");
    assert!(stdout.contains(HEADLINE), "{flag}: {stdout}");
    assert!(stdout.contains(REPOSITORY), "{flag}: {stdout}");
    if flag == "--help" {
      assert!(stdout.contains("Usage: vpdf"), "{stdout}");
    } else {
      assert!(stdout.starts_with(HEADLINE), "{stdout}");
      assert!(stdout.lines().last().unwrap().starts_with("vpdf "), "{stdout}");
    }
  }
  Ok(())
}

#[test]
fn preview_takes_one_pdf() -> Result<()> {
  let output = Command::new(env!("CARGO_BIN_EXE_vpdf"))
    .env(vpdf::inline::VAR, "text")
    .args(["a.pdf", "b.pdf"])
    .output()?;
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(!output.status.success(), "{stderr}");
  assert!(stderr.contains("Preview takes one PDF"), "{stderr}");
  Ok(())
}

#[test]
fn version_text_matches_cli() -> Result<()> {
  for flag in ["--version", "-V"] {
    let output = Command::new(env!("CARGO_BIN_EXE_vpdf")).arg(flag).output()?;
    assert!(output.status.success());
    assert_eq!(
      String::from_utf8(output.stdout)?,
      format!("{}\n", vpdf::version::text(true))
    );
  }
  Ok(())
}

#[test]
#[cfg(unix)]
fn output_handles_closed_pipe() -> Result<()> {
  use std::os::fd::OwnedFd;
  use std::os::unix::net::UnixStream;
  use std::process::Stdio;

  for flag in ["--version", "--config-example"] {
    let (reader, writer) = UnixStream::pair()?;
    drop(reader);
    let output = Command::new(env!("CARGO_BIN_EXE_vpdf"))
      .arg(flag)
      .stdout(Stdio::from(OwnedFd::from(writer)))
      .output()?;
    assert!(output.status.success(), "{flag}: {:?}", output.status);
    assert!(output.stderr.is_empty(), "{flag}: {:?}", output.stderr);
  }
  Ok(())
}

#[test]
fn config_output_skips_files() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("broken.toml");
  std::fs::write(&path, "[broken")?;
  for flag in ["--help", "--version", "--config-example"] {
    let output = Command::new(env!("CARGO_BIN_EXE_vpdf"))
      .arg("--config")
      .arg(&path)
      .arg(flag)
      .output()?;
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    if flag == "--config-example" {
      assert_eq!(
        String::from_utf8(output.stdout)?,
        vpdf::config::Settings::example()?
      );
    }
  }
  let output = Command::new(env!("CARGO_BIN_EXE_vpdf"))
    .args(["--config", "missing", "--no-config"])
    .output()?;
  assert!(!output.status.success());
  assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used"));
  Ok(())
}

#[test]
fn config_errors_reach_frontend() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("broken.toml");
  std::fs::write(&path, "[reading]\nzoom = 2\n[broken")?;
  let output = Command::new(env!("CARGO_BIN_EXE_vpdf"))
    .env_remove(vpdf::inline::VAR)
    .arg("--config")
    .arg(path)
    .output()?;
  assert!(!output.status.success());
  assert!(
    String::from_utf8_lossy(&output.stderr).contains("TUI requires a terminal")
  );
  Ok(())
}

#[test]
#[cfg(unix)]
fn config_bypass_skips_reads() -> Result<()> {
  use std::process::Stdio;
  use std::time::{Duration, Instant};

  let directory = tempfile::tempdir()?;
  std::fs::create_dir(directory.path().join("vpdf"))?;
  let path = directory.path().join("vpdf/config.toml");
  assert!(Command::new("mkfifo").arg(path).status()?.success());
  let mut child = Command::new(env!("CARGO_BIN_EXE_vpdf"))
    .env_remove(vpdf::inline::VAR)
    .env("XDG_CONFIG_HOME", directory.path())
    .arg("--no-config")
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;
  let deadline = Instant::now() + Duration::from_secs(3);
  while child.try_wait()?.is_none() {
    if Instant::now() >= deadline {
      child.kill()?;
      child.wait()?;
      anyhow::bail!("Configuration bypass read the FIFO");
    }
    std::thread::sleep(Duration::from_millis(10));
  }
  let output = child.wait_with_output()?;
  assert!(
    String::from_utf8_lossy(&output.stderr).contains("TUI requires a terminal")
  );
  Ok(())
}
