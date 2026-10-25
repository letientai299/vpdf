/// Keeps one collection face's tables unchanged.
pub fn compact(bytes: &[u8], index: u32) -> Option<Vec<u8>> {
  if bytes.get(..4)? != b"ttcf" || index as usize >= word(bytes, 8)? {
    return None;
  }
  let slot = (index as usize).checked_mul(4)?.checked_add(12)?;
  let face = bytes.get(word(bytes, slot)?..)?;
  let header = face.get(..12)?;
  let count = usize::from(u16::from_be_bytes(header[4..6].try_into().ok()?));
  if count == 0 {
    return None;
  }
  let records = face.get(12..12 + count * 16)?;
  let length = records.as_chunks::<16>().0.iter().try_fold(
    28 + records.len(),
    |size, record| {
      if &record[..4] == b"DSIG" {
        return None;
      }
      let table = bytes.get(word(record, 8)?..)?.get(..word(record, 12)?)?;
      size.checked_add(table.len().checked_add(3)? & !3)
    },
  )?;
  if length >= bytes.len() {
    return None;
  }
  let mut packed = Vec::with_capacity(length);
  // Collection heads ignore checksumAdjustment.
  packed.extend_from_slice(b"ttcf\0\x01\0\0\0\0\0\x01\0\0\0\x10");
  packed.extend_from_slice(header);
  packed.extend_from_slice(records);
  for (index, record) in records.as_chunks::<16>().0.iter().enumerate() {
    let table = bytes.get(word(record, 8)?..)?.get(..word(record, 12)?)?;
    let offset = u32::try_from(packed.len()).ok()?;
    let target = 28 + index * 16 + 8;
    packed[target..target + 4].copy_from_slice(&offset.to_be_bytes());
    packed.extend_from_slice(table);
    packed.resize((packed.len() + 3) & !3, 0);
  }
  Some(packed)
}

fn word(bytes: &[u8], offset: usize) -> Option<usize> {
  let value = bytes.get(offset..)?.first_chunk::<4>()?;
  usize::try_from(u32::from_be_bytes(*value)).ok()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rejects_truncated_collections() {
    for bytes in [&b""[..], b"ttcf", b"ttcf\0\x01\0\0\xff\xff\xff\xff"] {
      assert!(compact(bytes, 0).is_none());
      assert!(compact(bytes, u32::MAX).is_none());
    }
  }
}
