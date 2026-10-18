//! Search records on disk, so reopening a PDF skips its first full scan.
//!
//! One pack per PDF, named by the SHA-256 of its bytes, holds a
//! compressed `Index` record per page. Packs live where the OS may purge
//! them; a missing, short, damaged, or stale pack reads as a miss.
use crate::Result;
use crate::document::{GLYPH_LIMIT, MIB, TEXT_FLAGS};
use crate::search::Index;
use crate::text::PageId;
use anyhow::{Context, ensure};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

const MAGIC: &[u8; 8] = b"VPDFIDX1";
/// Packs kept before the least recently used go.
const BUDGET: u64 = 1 << 30;
/// Free disk space below which nothing is written.
const RESERVE: u64 = 2 << 30;
/// Largest decoded page record; a page holds at most `GLYPH_LIMIT`
/// glyphs.
const RECORD: u64 = 16 * MIB as u64;
/// Age after which a temporary pack counts as abandoned.
const ABANDONED: Duration = Duration::from_secs(3600);

pub type Alias = [u8; 32];

/// Writes so far, naming each writer's temporary pack.
static WRITES: AtomicU64 = AtomicU64::new(0);

/// Where packs live: the user caches directory on macOS, which the OS
/// may purge. `VPDF_CACHE` names another directory on any system, or
/// `off` disables the cache. Without it, other systems stay off until a
/// location with OS cleanup is chosen.
pub fn root() -> Option<PathBuf> {
  // A build that could not name its index crates cannot tell packs apart.
  if env!("VPDF_INDEX_CRATES").is_empty() {
    return None;
  }
  match std::env::var_os("VPDF_CACHE") {
    Some(dir) if dir.is_empty() || dir == "off" => None,
    Some(dir) => Some(PathBuf::from(dir)),
    None if cfg!(target_os = "macos") => {
      let home = PathBuf::from(std::env::var_os("HOME")?);
      Some(home.join("Library/Caches/vpdf/content"))
    }
    None => None,
  }
}

/// SHA-256 of the bytes of `file`.
pub fn alias(file: &mut File) -> Result<Alias> {
  let mut hasher = Sha256::new();
  let mut buffer = vec![0; MIB];
  loop {
    match file.read(&mut buffer)? {
      0 => return Ok(hasher.finalize().into()),
      read => hasher.update(&buffer[..read]),
    }
  }
}

/// What shaped the records: a change makes every pack stale.
fn provenance() -> String {
  format!(
    "schema 1; flags {}; glyphs {GLYPH_LIMIT}; code {}; {}",
    TEXT_FLAGS.bits(),
    env!("VPDF_INDEX_CODE"),
    env!("VPDF_INDEX_CRATES")
  )
}

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
  bytes.iter().fold(String::new(), |mut out, byte| {
    // Writing to a String cannot fail.
    let _ = write!(out, "{byte:02x}");
    out
  })
}

fn path(root: &Path, alias: &Alias) -> PathBuf {
  root.join(format!("{}.pack", hex(alias)))
}

/// A pack opened for reading. Its open file keeps answering after the
/// OS deletes the path.
pub struct Pack {
  file: File,
  /// Offset and length of each page's record; zero length when absent.
  table: Vec<(u64, u32)>,
}

impl Pack {
  /// The valid pack for `alias` with `pages` pages, else none.
  pub fn open(root: &Path, alias: &Alias, pages: usize) -> Option<Self> {
    let file = File::open(path(root, alias)).ok()?;
    let pack = Self::read(file, pages).ok()?;
    // Recency for eviction.
    drop(pack.file.set_modified(SystemTime::now()));
    Some(pack)
  }

  fn read(file: File, pages: usize) -> Result<Self> {
    let len = file.metadata()?.len();
    let mut at = 0;
    let mut take = |count: usize| -> Result<Vec<u8>> {
      let mut bytes = vec![0; count];
      file.read_exact_at(&mut bytes, at)?;
      at += count as u64;
      Ok(bytes)
    };
    ensure!(take(MAGIC.len())? == MAGIC, "Not a search pack");
    let expected = provenance();
    let size = u32_at(&take(4)?, 0) as usize;
    ensure!(size == expected.len(), "Stale search pack");
    ensure!(take(size)? == expected.as_bytes(), "Stale search pack");
    ensure!(u32_at(&take(4)?, 0) as usize == pages, "Search pack page count");
    let raw = take(pages.checked_mul(12).context("Search pack size")?)?;
    let table: Vec<(u64, u32)> = raw
      .as_chunks::<12>()
      .0
      .iter()
      .map(|entry| {
        let low = u64::from(u32_at(entry, 0));
        let high = u64::from(u32_at(entry, 4));
        (high << 32 | low, u32_at(entry, 8))
      })
      .collect();
    let fits = |&(offset, size): &(u64, u32)| {
      u64::from(size) <= RECORD
        && offset.checked_add(u64::from(size)).is_some_and(|end| end <= len)
    };
    ensure!(table.iter().all(fits), "Truncated search pack");
    Ok(Self { file, table })
  }

  /// `page`'s index; none when absent or damaged.
  pub fn index(&self, page: PageId) -> Option<Index> {
    let raw = self.raw(page)?;
    let mut bytes = Vec::new();
    let mut decoder = ZlibDecoder::new(raw.as_slice()).take(RECORD);
    decoder.read_to_end(&mut bytes).ok()?;
    Index::decode(page, &bytes).ok()
  }

  fn holds(&self, page: PageId) -> bool {
    self.table.get(page.0).is_some_and(|&(_, size)| size > 0)
  }

  fn raw(&self, page: PageId) -> Option<Vec<u8>> {
    let &(offset, size) = self.table.get(page.0)?;
    if size == 0 {
      return None;
    }
    let mut raw = vec![0; size as usize];
    self.file.read_exact_at(&mut raw, offset).ok()?;
    Some(raw)
  }
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
  u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Compressed page records waiting for a pack.
pub struct Records(Vec<Option<Vec<u8>>>);

impl Records {
  pub fn new(pages: usize) -> Self {
    Self(vec![None; pages])
  }

  pub fn add(&mut self, page: PageId, index: &Index) -> Result<()> {
    let slot = self.0.get_mut(page.0).context("Page outside document")?;
    let mut bytes = Vec::new();
    index.encode(&mut bytes);
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&bytes)?;
    *slot = Some(encoder.finish()?);
    Ok(())
  }

  pub fn is_empty(&self) -> bool {
    self.0.iter().all(Option::is_none)
  }

  /// Whether `old` lacks a page recorded here.
  pub fn adds_to(&self, old: Option<&Pack>) -> bool {
    self.0.iter().enumerate().any(|(page, raw)| {
      raw.is_some() && !old.is_some_and(|old| old.holds(PageId(page)))
    })
  }
}

/// Writes the pack for `alias` from `records`, keeping `old`'s records
/// of other pages, then trims the cache to its budget. Skips the write
/// when the disk is nearly full.
pub fn save(
  root: &Path,
  alias: &Alias,
  records: &Records,
  old: Option<&Pack>,
) -> Result<()> {
  // Pages hold the PDF's text: only its owner reads them.
  std::fs::DirBuilder::new().recursive(true).mode(0o700).create(root)?;
  let stats = rustix::fs::statvfs(root)?;
  if stats.f_bavail.saturating_mul(stats.f_frsize) < RESERVE {
    return Ok(());
  }
  let raws: Vec<Option<Vec<u8>>> = records
    .0
    .iter()
    .enumerate()
    .map(|(page, raw)| {
      raw.clone().or_else(|| old.and_then(|old| old.raw(PageId(page))))
    })
    .collect();
  let mut bytes = MAGIC.to_vec();
  let provenance = provenance();
  put(&mut bytes, provenance.len());
  bytes.extend_from_slice(provenance.as_bytes());
  put(&mut bytes, raws.len());
  let mut offset = (bytes.len() + raws.len() * 12) as u64;
  for raw in &raws {
    let size = raw.as_ref().map_or(0, Vec::len);
    let start = if size == 0 { 0 } else { offset };
    bytes.extend_from_slice(&(start as u32).to_le_bytes());
    bytes.extend_from_slice(&((start >> 32) as u32).to_le_bytes());
    put(&mut bytes, size);
    offset += size as u64;
  }
  for raw in raws.iter().flatten() {
    bytes.extend_from_slice(raw);
  }
  // Writers race by renaming whole packs; readers never see a partial
  // one.
  let write = WRITES.fetch_add(1, Ordering::Relaxed);
  let target = path(root, alias);
  let temp =
    target.with_extension(format!("{}-{write}.tmp", std::process::id()));
  let written = File::options()
    .write(true)
    .create_new(true)
    .mode(0o600)
    .open(&temp)
    .and_then(|mut file| file.write_all(&bytes))
    .and_then(|()| std::fs::rename(&temp, &target));
  if let Err(error) = written {
    drop(std::fs::remove_file(&temp));
    return Err(error.into());
  }
  trim(root, BUDGET)
}

fn put(bytes: &mut Vec<u8>, value: usize) {
  bytes.extend_from_slice(&(value as u32).to_le_bytes());
}

/// Deletes the least recently used packs until the rest fit `budget`,
/// and temporary packs a crashed writer left.
fn trim(root: &Path, budget: u64) -> Result<()> {
  let mut packs = Vec::new();
  let now = SystemTime::now();
  for entry in std::fs::read_dir(root)?.flatten() {
    let Ok(meta) = entry.metadata() else { continue };
    let modified = meta.modified().unwrap_or(now);
    let path = entry.path();
    match path.extension().and_then(|extension| extension.to_str()) {
      Some("pack") => packs.push((modified, meta.len(), path)),
      Some("tmp")
        if now.duration_since(modified).unwrap_or_default() > ABANDONED =>
      {
        drop(std::fs::remove_file(path));
      }
      _ => {}
    }
  }
  let mut total: u64 = packs.iter().map(|(_, len, _)| len).sum();
  packs.sort_by_key(|(modified, ..)| *modified);
  for (_, len, path) in packs {
    if total <= budget {
      break;
    }
    if std::fs::remove_file(path).is_ok() {
      total -= len;
    }
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::search::compile;
  use crate::text::{Bounds, Glyph, Line, Text};

  fn index(page: usize, words: &str) -> Index {
    let line = Line {
      paragraph: true,
      glyphs: words
        .chars()
        .map(|ch| Glyph { ch, bounds: Bounds::default() })
        .collect(),
    };
    Index::new(PageId(page), &Text { lines: vec![line], truncated: false })
  }

  fn hits(index: &Index, query: &str) -> usize {
    let pattern = compile(query, false).expect("query");
    index.matches(&pattern, 100).expect("matches").len()
  }

  fn records(pages: usize, entries: &[(usize, &str)]) -> Records {
    let mut records = Records::new(pages);
    for (page, words) in entries {
      records.add(PageId(*page), &index(*page, words)).expect("record");
    }
    records
  }

  /// Saved pages read back; absent pages, other page counts, and other
  /// aliases miss; a later save keeps earlier pages.
  #[test]
  fn packs_round_trip_and_merge() -> Result<()> {
    let root = tempfile::tempdir()?;
    let alias = [7; 32];
    save(root.path(), &alias, &records(3, &[(0, "alpha beta")]), None)?;
    let pack = Pack::open(root.path(), &alias, 3).context("pack")?;
    assert_eq!(hits(&pack.index(PageId(0)).context("page 0")?, "beta"), 1);
    assert!(pack.index(PageId(1)).is_none());
    assert!(Pack::open(root.path(), &alias, 4).is_none());
    assert!(Pack::open(root.path(), &[8; 32], 3).is_none());
    save(root.path(), &alias, &records(3, &[(2, "gamma")]), Some(&pack))?;
    let pack = Pack::open(root.path(), &alias, 3).context("merged")?;
    assert_eq!(hits(&pack.index(PageId(0)).context("kept")?, "alpha"), 1);
    assert_eq!(hits(&pack.index(PageId(2)).context("added")?, "gamma"), 1);
    Ok(())
  }

  /// Damage anywhere reads as a miss, never a panic; an open pack keeps
  /// answering after its file is deleted.
  #[test]
  fn damaged_packs_miss() -> Result<()> {
    let root = tempfile::tempdir()?;
    let alias = [1; 32];
    save(root.path(), &alias, &records(2, &[(0, "one"), (1, "two")]), None)?;
    let file = path(root.path(), &alias);
    let bytes = std::fs::read(&file)?;
    for len in (0..bytes.len()).step_by(3) {
      std::fs::write(&file, &bytes[..len])?;
      let pack = Pack::open(root.path(), &alias, 2);
      let read =
        pack.map(|pack| (pack.index(PageId(0)), pack.index(PageId(1))));
      assert!(!matches!(read, Some((Some(_), Some(_)))), "{len}");
    }
    let mut stale = bytes.clone();
    stale[MAGIC.len() + 4] ^= 1;
    std::fs::write(&file, &stale)?;
    assert!(Pack::open(root.path(), &alias, 2).is_none(), "provenance");
    let mut flipped = bytes.clone();
    let last = flipped.len() - 2;
    flipped[last] ^= 0xff;
    std::fs::write(&file, &flipped)?;
    let pack = Pack::open(root.path(), &alias, 2).context("table intact")?;
    assert!(pack.index(PageId(1)).is_none(), "damaged record");
    std::fs::write(&file, &bytes)?;
    let pack = Pack::open(root.path(), &alias, 2).context("restored")?;
    std::fs::remove_file(&file)?;
    assert_eq!(hits(&pack.index(PageId(1)).context("deleted")?, "two"), 1);
    assert!(Pack::open(root.path(), &alias, 2).is_none());
    Ok(())
  }

  /// Trimming keeps the most recently used packs within the budget and
  /// leaves fresh temporary packs to their writers.
  #[test]
  fn trim_drops_least_recent() -> Result<()> {
    let root = tempfile::tempdir()?;
    let aliases = [[1; 32], [2; 32], [3; 32]];
    for (age, alias) in aliases.iter().enumerate() {
      save(root.path(), alias, &records(1, &[(0, "words")]), None)?;
      let modified =
        SystemTime::now() - Duration::from_secs(60 * (3 - age as u64));
      File::options()
        .write(true)
        .open(path(root.path(), alias))?
        .set_modified(modified)?;
    }
    let temp = root.path().join("writer.tmp");
    std::fs::write(&temp, b"partial")?;
    let abandoned = root.path().join("crashed.tmp");
    std::fs::write(&abandoned, b"partial")?;
    let old = SystemTime::now() - ABANDONED * 2;
    File::options().write(true).open(&abandoned)?.set_modified(old)?;
    let size = std::fs::metadata(path(root.path(), &aliases[0]))?.len();
    trim(root.path(), size * 2)?;
    let kept: Vec<bool> =
      aliases.iter().map(|alias| path(root.path(), alias).exists()).collect();
    assert_eq!(kept, [false, true, true]);
    assert!(temp.exists());
    assert!(!abandoned.exists());
    Ok(())
  }

  /// Reading a pack marks it recently used.
  #[test]
  fn reading_refreshes_recency() -> Result<()> {
    let root = tempfile::tempdir()?;
    let alias = [4; 32];
    save(root.path(), &alias, &records(1, &[(0, "words")]), None)?;
    let file = path(root.path(), &alias);
    let old = SystemTime::now() - Duration::from_secs(3600);
    File::options().write(true).open(&file)?.set_modified(old)?;
    Pack::open(root.path(), &alias, 1).context("pack")?;
    assert!(std::fs::metadata(&file)?.modified()? > old);
    Ok(())
  }

  /// Writers racing on one alias leave one whole pack.
  #[test]
  fn racing_writers_leave_one_pack() -> Result<()> {
    let root = tempfile::tempdir()?;
    let alias = [9; 32];
    let words = ["red", "green", "blue", "cyan"];
    std::thread::scope(|scope| {
      for word in words {
        let root = root.path();
        let records = records(1, &[(0, word)]);
        scope.spawn(move || {
          (0..10).try_for_each(|_| save(root, &alias, &records, None))
        });
      }
    });
    let pack = Pack::open(root.path(), &alias, 1).context("pack")?;
    let page = pack.index(PageId(0)).context("page")?;
    assert_eq!(words.iter().map(|word| hits(&page, word)).sum::<usize>(), 1);
    let files = std::fs::read_dir(root.path())?.count();
    assert_eq!(files, 1, "temporary packs remain");
    Ok(())
  }
}
