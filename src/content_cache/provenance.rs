use crate::document::{GLYPH_LIMIT, TEXT_FLAGS};

/// What shaped the records: a change makes every pack stale.
pub(super) fn provenance() -> String {
  format!(
    "schema 2; flags {}; glyphs {GLYPH_LIMIT}; code {}; {}",
    TEXT_FLAGS.bits(),
    env!("VPDF_INDEX_CODE"),
    env!("VPDF_INDEX_CRATES")
  )
}
