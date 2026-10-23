use std::{env, fs, path::PathBuf, process::Command};

pub fn prepare() {
  let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output"));
  let facts = output.join("help-facts.json");
  fs::write(
    &facts,
    vpdf_ui::topics::build::facts().expect("Help reference facts").to_string(),
  )
  .expect("Write help facts");
  println!("cargo::rerun-if-env-changed=CARGO_TARGET_DIR");
  println!("cargo::rerun-if-changed=docs/users");
  let result = Command::new("python3")
    .args(["-B", "scripts/help_assets.py", "--check", "--facts"])
    .arg(facts)
    .output()
    .expect("Help verification needs Python 3; run mise run help:build");
  assert!(
    result.status.success(),
    "{}",
    String::from_utf8_lossy(&result.stderr)
  );
  let directory = String::from_utf8(result.stdout).expect("Help asset path");
  let directory = directory.trim();
  println!("cargo::rerun-if-changed={directory}");
  println!("cargo::rustc-env=VPDF_HELP_DIR={directory}");
}
