use std::process::Command;
use vpdf::Result;

const HEADLINE: &str = concat!(
  "A Vim-inspired terminal PDF reader powered by MuPDF ",
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
