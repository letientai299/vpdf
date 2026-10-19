use anyhow::Result;
use std::hint::black_box;
use std::io::Write;
#[path = "../../../benches/support/mod.rs"]
mod support;
use support::measure;

/// One terminal read: pointer motion, keys, and a graphics reply, as
/// kitty sends them while the reader scrolls.
fn decode() -> Result<()> {
  use std::collections::VecDeque;
  use vpdf_protocol::decode::Parser;
  let mut read = Vec::new();
  for column in 0..64 {
    write!(read, "\x1b[<35;{};14M", column + 1)?;
  }
  read.extend_from_slice(b"jjk\x1b[105;5u\x1b_Gi=31;OK\x1b\\");
  let mut parser = Parser::default();
  let mut events = VecDeque::new();
  measure("Parser::feed 69 events", || {
    parser.feed(black_box(&read), &mut events);
    events.clear();
  })
}

fn main() -> Result<()> {
  if support::selected("decode") {
    decode()?;
  }
  Ok(())
}
