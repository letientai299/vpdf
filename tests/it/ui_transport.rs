mod chrome;

/// Output that fails every write.
struct Closed;

impl std::io::Write for Closed {
  fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
    Err(std::io::Error::other("Closed output"))
  }
  fn flush(&mut self) -> std::io::Result<()> {
    Ok(())
  }
}
mod hints;
mod home;
mod layout_tiles;
mod overview;
mod page;
mod paint;
mod panel;
mod prompt;
mod region;
mod scrollbar;
mod split;
