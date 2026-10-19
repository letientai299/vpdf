use super::{Cow, Engine, Read, Result, STANDARD, Write, ZlibDecoder};

pub(super) fn transmit(
  out: &mut impl Write,
  id: u32,
  width: i32,
  height: i32,
  format: u8,
  encoded: &str,
) -> Result<()> {
  // iTerm2 rejects short compressed payloads.
  let (compression, encoded) = if encoded.len() <= 20 {
    let compressed = STANDARD.decode(encoded)?;
    let mut pixels = Vec::new();
    ZlibDecoder::new(compressed.as_slice()).read_to_end(&mut pixels)?;
    ("", Cow::Owned(STANDARD.encode(pixels)))
  } else {
    ("o=z,", Cow::Borrowed(encoded))
  };
  let control =
    format!("a=t,f={format},{compression}i={id},q=2,s={width},v={height}");
  crate::terminal::transmit(out, &control, &encoded)
}
