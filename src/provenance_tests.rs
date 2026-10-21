use crate::provenance::Source;
use std::process::Command;

#[test]
fn checkout_identity_is_local() -> crate::Result<()> {
  let folder = tempfile::tempdir()?;
  assert!(Source::read(folder.path()).is_none());
  assert!(Source::read_using(folder.path(), "/missing/git").is_none());
  for args in [
    vec!["init", "-q"],
    vec![
      "-c",
      "user.name=test",
      "-c",
      "user.email=test@example.invalid",
      "commit",
      "-qm",
      "initial",
      "--allow-empty",
    ],
  ] {
    assert!(
      Command::new("git")
        .args(args)
        .current_dir(folder.path())
        .status()?
        .success()
    );
  }
  let clean = Source::read(folder.path()).unwrap();
  assert!(!clean.dirty);
  std::fs::write(folder.path().join("tracked"), "initial")?;
  assert!(
    Command::new("git")
      .args(["add", "tracked"])
      .current_dir(folder.path())
      .status()?
      .success()
  );
  std::fs::write(folder.path().join("tracked"), "changed")?;
  std::fs::write(folder.path().join("source"), "changed")?;
  std::fs::create_dir(folder.path().join("nested"))?;
  let dirty = Source::read(folder.path()).unwrap();
  assert!(dirty.dirty);
  assert_eq!(clean.revision, dirty.revision);
  assert_ne!(clean.patch, dirty.patch);
  let nested = Source::read(&folder.path().join("nested")).unwrap();
  assert_eq!(dirty.revision, nested.revision);
  assert_eq!(dirty.dirty, nested.dirty);
  assert_eq!(dirty.patch, nested.patch);
  Ok(())
}
