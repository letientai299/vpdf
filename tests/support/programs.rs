use crate::Result;
use std::os::unix::fs::PermissionsExt;

pub fn picker_paths(test: &str) -> Result<bool> {
  if std::env::var_os("VPDF_TEST_PICKER").is_some() {
    return Ok(false);
  }
  let root = tempfile::tempdir()?;
  for present in [false, true] {
    if present {
      let helper = root.path().join("fzf");
      std::fs::write(&helper, "#!/bin/sh\nexit 1\n")?;
      std::fs::set_permissions(helper, std::fs::Permissions::from_mode(0o755))?;
    }
    let output = std::process::Command::new(std::env::current_exe()?)
      .args(["--exact", test, "--nocapture"])
      .env("PATH", root.path())
      .env("VPDF_TEST_PICKER", "1")
      .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::ensure!(
      output.status.success() && stdout.contains("1 passed;"),
      "Picker routing failed: fzf={present}\n{stdout}\n{stderr}"
    );
  }
  Ok(true)
}
