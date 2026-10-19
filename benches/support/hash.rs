use sha2::{Digest, Sha256};
use std::fmt::Write;

pub fn digest(bytes: &[u8]) -> String {
  Sha256::digest(bytes).iter().fold(
    String::with_capacity(64),
    |mut text, byte| {
      write!(text, "{byte:02x}").expect("hex string");
      text
    },
  )
}
