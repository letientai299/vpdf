/// Cargo runs benches from the package root; relative
/// inputs and outputs name workspace paths.
pub fn enter() -> std::io::Result<()> {
  let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
  std::env::set_current_dir(root).map_err(|error| {
    std::io::Error::new(error.kind(), format!("Workspace root {root}: {error}"))
  })
}
