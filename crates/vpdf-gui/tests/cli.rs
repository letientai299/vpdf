use std::process::Command;

#[test]
fn version_text_matches_cli() -> Result<(), Box<dyn std::error::Error>> {
  for flag in ["--version", "-V"] {
    let output =
      Command::new(env!("CARGO_BIN_EXE_vpdf-gui")).arg(flag).output()?;
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout)?;
    assert_eq!(stdout, format!("{}\n", vpdf::version::text(false)));
    assert!(stdout.starts_with(vpdf::version::ABOUT));
    assert_eq!(
      stdout.lines().last(),
      Some(format!("vpdf-gui {}", vpdf::version::BUILD).as_str())
    );
  }
  Ok(())
}
