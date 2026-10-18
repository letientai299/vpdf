//! Search records on disk, so reopening a PDF skips its first full scan.
//!
//! One pack per PDF, named by the SHA-256 of its bytes, holds a
//! compressed `Index` record per page. Packs live where the OS may purge
//! them; a missing, short, damaged, or stale pack reads as a miss. Where
//! file versions are exact, a memo per version remembers its hash, so a
//! reopen need not hash. OCR output lives beside the packs, named by a
//! hash of the raster and the engine that read it.
use crate::Result;
use crate::document::{GLYPH_LIMIT, MIB, TEXT_FLAGS};
use crate::search::Index;
use crate::text::PageId;
use anyhow::{Context, ensure};
use flate2::write::ZlibEncoder;
use flate2::{Compression, Decompress, FlushDecompress, Status};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs::{File, Metadata};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

const MAGIC: &[u8; 8] = b"VPDFIDX1";
const OCR_MAGIC: &[u8; 8] = b"VPDFOCR1";
/// Packs and OCR output kept before the least recently used go.
const BUDGET: u64 = 1 << 30;
/// Free disk space below which nothing is written.
const RESERVE: u64 = 2 << 30;
/// Largest decoded page record; a page holds at most `GLYPH_LIMIT`
/// glyphs.
const RECORD: u64 = 16 * MIB as u64;
/// Age after which a temporary pack counts as abandoned.
const ABANDONED: Duration = Duration::from_secs(3600);
/// Age after which an unused memo goes.
const FORGOTTEN: Duration = Duration::from_hours(30 * 24);
/// Table bytes per page: offset, stored size, decoded size.
const ENTRY: usize = 16;
/// Fewest pages a decoder thread takes; smaller batches decode on the
/// calling thread.
const SHARE: usize = 16;

pub type Alias = [u8; 32];

/// A file version as `stat` tells it: device, inode, size, and
/// modification and change times. A hint, except on filesystems whose
/// times are exact (`exact`): a write within one coarse tick, or cached
/// network attributes, can keep it unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Version([u64; 7]);

impl Version {
  pub fn of(meta: &Metadata) -> Self {
    Self([
      meta.dev(),
      meta.ino(),
      meta.len(),
      meta.mtime().cast_unsigned(),
      meta.mtime_nsec().cast_unsigned(),
      meta.ctime().cast_unsigned(),
      meta.ctime_nsec().cast_unsigned(),
    ])
  }

  /// `path`'s version now; none when it cannot be read.
  pub fn read(path: &Path) -> Option<Self> {
    std::fs::metadata(path).ok().map(|meta| Self::of(&meta))
  }

  fn bytes(self) -> Vec<u8> {
    self.0.iter().flat_map(|field| field.to_le_bytes()).collect()
  }
}

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
    "schema 2; flags {}; glyphs {GLYPH_LIMIT}; code {}; {}",
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

/// Whether `path` lies on a filesystem with nanosecond change times and
/// unique inodes, where a matching `Version` means unchanged bytes: APFS.
/// Elsewhere a reopen hashes.
pub fn exact(path: &Path) -> bool {
  #[cfg(target_os = "macos")]
  {
    rustix::fs::statfs(path).is_ok_and(|stats| {
      let name = stats.f_fstypename.map(|byte| byte as u8);
      name.starts_with(b"apfs\0")
    })
  }
  #[cfg(not(target_os = "macos"))]
  {
    let _ = path;
    false
  }
}

/// One memo per file, so a new version replaces the old one's.
fn memo(root: &Path, version: Version) -> PathBuf {
  root.join(format!("{:x}-{:x}.id", version.0[0], version.0[1]))
}

/// The alias remembered for the file version `version` names.
pub fn recall(root: &Path, version: Version) -> Option<Alias> {
  let path = memo(root, version);
  let bytes = std::fs::read(&path).ok()?;
  let (head, alias) = bytes.split_at_checked(bytes.len().checked_sub(32)?)?;
  let alias = (head == version.bytes()).then(|| alias.try_into().ok())??;
  touch(&path);
  Some(alias)
}

/// Remembers `alias` as the hash of the file version `version` names.
pub fn remember(root: &Path, version: Version, alias: &Alias) -> Result<()> {
  create(root)?;
  let mut bytes = version.bytes();
  bytes.extend_from_slice(alias);
  publish(&memo(root, version), &bytes)
}

/// Makes `root` and tells whether it has room to write.
fn room(root: &Path) -> Result<bool> {
  create(root)?;
  let stats = rustix::fs::statvfs(root)?;
  Ok(stats.f_bavail.saturating_mul(stats.f_frsize) >= RESERVE)
}

/// OCR output `key` names; none when absent or damaged.
pub fn recall_ocr(root: &Path, key: &Alias) -> Option<String> {
  let path = ocr_path(root, key);
  let header = (OCR_MAGIC.len() + 4) as u64;
  if std::fs::metadata(&path).ok()?.len() > RECORD + header {
    return None;
  }
  let bytes = std::fs::read(&path).ok()?;
  let rest = bytes.strip_prefix(OCR_MAGIC)?;
  let (len, raw) = rest.split_first_chunk::<4>()?;
  let len = u32::from_le_bytes(*len) as usize;
  if u64::try_from(len).ok()? > RECORD {
    return None;
  }
  let text = String::from_utf8(inflate(raw, len)?).ok()?;
  touch(&path);
  Some(text)
}

/// Stores OCR output under `key`, then trims the cache to its budget.
pub fn remember_ocr(root: &Path, key: &Alias, text: &str) -> Result<()> {
  // Longer output would never read back.
  if text.len() as u64 > RECORD || !room(root)? {
    return Ok(());
  }
  let mut bytes = OCR_MAGIC.to_vec();
  put(&mut bytes, text.len());
  bytes.extend_from_slice(&deflate(text.as_bytes())?);
  publish(&ocr_path(root, key), &bytes)?;
  trim(root, BUDGET)
}

/// Marks `path` recently used, for trimming.
fn touch(path: &Path) {
  drop(File::open(path).and_then(|file| file.set_modified(SystemTime::now())));
}

fn ocr_path(root: &Path, key: &Alias) -> PathBuf {
  root.join(format!("{}.ocr", hex(key)))
}

/// Makes `root`; pages hold the PDF's text, so only its owner reads them.
fn create(root: &Path) -> Result<()> {
  std::fs::DirBuilder::new().recursive(true).mode(0o700).create(root)?;
  Ok(())
}

/// Writes `bytes` to `target` whole: writers race by renaming whole
/// files, so readers never see a partial one.
fn publish(target: &Path, bytes: &[u8]) -> Result<()> {
  let write = WRITES.fetch_add(1, Ordering::Relaxed);
  let temp =
    target.with_extension(format!("{}-{write}.tmp", std::process::id()));
  let written = File::options()
    .write(true)
    .create_new(true)
    .mode(0o600)
    .open(&temp)
    .and_then(|mut file| file.write_all(bytes))
    .and_then(|()| std::fs::rename(&temp, target));
  if let Err(error) = written {
    drop(std::fs::remove_file(&temp));
    return Err(error.into());
  }
  Ok(())
}

/// A pack opened for reading. Its open file keeps answering after the
/// OS deletes the path.
pub struct Pack {
  file: File,
  table: Vec<Entry>,
  /// Held pages whose records failed to decode, so a save replaces them.
  damaged: Mutex<Vec<PageId>>,
}

/// Where a page's record lies and how long it decodes; zero size when
/// the page is absent.
#[derive(Clone, Copy)]
struct Entry {
  offset: u64,
  size: u32,
  len: u32,
}

/// A compressed record and its decoded length.
#[derive(Clone)]
struct Record {
  raw: Vec<u8>,
  len: u32,
}

impl Pack {
  /// The valid pack for `alias`, else none.
  pub fn open(root: &Path, alias: &Alias) -> Option<Self> {
    let file = File::open(path(root, alias)).ok()?;
    let pack = Self::read(file).ok()?;
    // Recency for eviction.
    drop(pack.file.set_modified(SystemTime::now()));
    Some(pack)
  }

  /// Pages the pack covers: its document's page count.
  pub fn pages(&self) -> usize {
    self.table.len()
  }

  fn read(file: File) -> Result<Self> {
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
    let pages = u32_at(&take(4)?, 0) as usize;
    let table_len = pages.checked_mul(ENTRY).context("Search pack size")?;
    ensure!(table_len as u64 <= len, "Truncated search pack");
    let raw = take(table_len)?;
    let table: Vec<Entry> = raw
      .as_chunks::<ENTRY>()
      .0
      .iter()
      .map(|entry| {
        let low = u64::from(u32_at(entry, 0));
        let high = u64::from(u32_at(entry, 4));
        Entry {
          offset: high << 32 | low,
          size: u32_at(entry, 8),
          len: u32_at(entry, 12),
        }
      })
      .collect();
    let fits = |entry: &Entry| {
      u64::from(entry.size) <= RECORD
        && u64::from(entry.len) <= RECORD
        && (entry.offset.checked_add(u64::from(entry.size)))
          .is_some_and(|end| end <= len)
    };
    ensure!(table.iter().all(fits), "Truncated search pack");
    Ok(Self { file, table, damaged: Mutex::new(Vec::new()) })
  }

  /// `page`'s index; none when absent or damaged.
  pub fn index(&self, page: PageId) -> Option<Index> {
    let record = self.record(page)?;
    let index = decode(page, &record);
    if index.is_none() {
      self.damaged.lock().unwrap_or_else(PoisonError::into_inner).push(page);
    }
    index
  }

  /// Indexes of the leading `pages` the pack holds, in order, up to
  /// `budget` decoded bytes (at least one page), on a few threads.
  pub fn indexes(
    &self,
    pages: &[PageId],
    budget: usize,
  ) -> Vec<(PageId, Index)> {
    let mut used = 0;
    let pages: Vec<PageId> = pages
      .iter()
      .copied()
      .filter(|&page| self.holds(page))
      .take_while(|page| {
        let fits =
          used == 0 || used + self.table[page.0].len as usize <= budget;
        used += self.table[page.0].len as usize;
        fits
      })
      .collect();
    let threads = crate::render_worker::pool_size();
    let chunk = pages.len().div_ceil(threads).max(SHARE);
    let decode = |pages: &[PageId]| -> Vec<(PageId, Index)> {
      pages.iter().filter_map(|&page| Some((page, self.index(page)?))).collect()
    };
    let mut chunks = pages.chunks(chunk);
    let Some(first) = chunks.next() else { return Vec::new() };
    std::thread::scope(|scope| {
      let jobs: Vec<_> = chunks
        .map(|pages| {
          let job = std::thread::Builder::new()
            .spawn_scoped(scope, move || decode(pages));
          // Without a thread, the chunk decodes after the first.
          job.map_err(|_| pages)
        })
        .collect();
      let mut found = decode(first);
      for job in jobs {
        match job {
          // A panicking decoder leaves its pages to the PDF.
          Ok(job) => found.extend(job.join().unwrap_or_default()),
          Err(pages) => found.extend(decode(pages)),
        }
      }
      found
    })
  }

  /// Whether the pack has a record for `page` that has not failed.
  fn holds(&self, page: PageId) -> bool {
    self.table.get(page.0).is_some_and(|entry| entry.size > 0)
      && !self
        .damaged
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(&page)
  }

  fn record(&self, page: PageId) -> Option<Record> {
    if !self.holds(page) {
      return None;
    }
    let entry = self.table[page.0];
    let mut raw = vec![0; entry.size as usize];
    self.file.read_exact_at(&mut raw, entry.offset).ok()?;
    Some(Record { raw, len: entry.len })
  }
}

/// The index `record` holds for `page`; none unless it inflates to its
/// stated length and decodes.
fn decode(page: PageId, record: &Record) -> Option<Index> {
  Index::decode(page, &inflate(&record.raw, record.len as usize)?).ok()
}

/// `raw` inflated, when it holds exactly `len` bytes.
fn inflate(raw: &[u8], len: usize) -> Option<Vec<u8>> {
  // One spare byte lets a longer stream show itself.
  let mut bytes = Vec::with_capacity(len + 1);
  let status = Decompress::new(true)
    .decompress_vec(raw, &mut bytes, FlushDecompress::Finish)
    .ok()?;
  (status == Status::StreamEnd && bytes.len() == len).then_some(bytes)
}

fn deflate(bytes: &[u8]) -> Result<Vec<u8>> {
  let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
  encoder.write_all(bytes)?;
  Ok(encoder.finish()?)
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
  u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Compressed page records waiting for a pack.
pub struct Records(Vec<Option<Record>>);

impl Records {
  pub fn new(pages: usize) -> Self {
    Self(vec![None; pages])
  }

  pub fn add(&mut self, page: PageId, index: &Index) -> Result<()> {
    let slot = self.0.get_mut(page.0).context("Page outside document")?;
    let mut bytes = Vec::new();
    index.encode(&mut bytes);
    let len = u32::try_from(bytes.len())?;
    *slot = Some(Record { raw: deflate(&bytes)?, len });
    Ok(())
  }

  pub fn is_empty(&self) -> bool {
    self.0.iter().all(Option::is_none)
  }

  /// Whether `old` lacks a page recorded here.
  pub fn adds_to(&self, old: Option<&Pack>) -> bool {
    self.0.iter().enumerate().any(|(page, record)| {
      record.is_some() && !old.is_some_and(|old| old.holds(PageId(page)))
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
  if !room(root)? {
    return Ok(());
  }
  let kept: Vec<Option<Record>> = records
    .0
    .iter()
    .enumerate()
    .map(|(page, record)| {
      record.clone().or_else(|| old.and_then(|old| old.record(PageId(page))))
    })
    .collect();
  let mut bytes = MAGIC.to_vec();
  let provenance = provenance();
  put(&mut bytes, provenance.len());
  bytes.extend_from_slice(provenance.as_bytes());
  put(&mut bytes, kept.len());
  let mut offset = (bytes.len() + kept.len() * ENTRY) as u64;
  for record in &kept {
    let (size, len) = record.as_ref().map_or((0, 0), |r| (r.raw.len(), r.len));
    let start = if size == 0 { 0 } else { offset };
    bytes.extend_from_slice(&(start as u32).to_le_bytes());
    bytes.extend_from_slice(&((start >> 32) as u32).to_le_bytes());
    put(&mut bytes, size);
    bytes.extend_from_slice(&len.to_le_bytes());
    offset += size as u64;
  }
  for raw in kept.iter().flatten().map(|record| &record.raw) {
    bytes.extend_from_slice(raw);
  }
  publish(&path(root, alias), &bytes)?;
  trim(root, BUDGET)
}

fn put(bytes: &mut Vec<u8>, value: usize) {
  bytes.extend_from_slice(&(value as u32).to_le_bytes());
}

/// Deletes the least recently used packs and OCR output until the rest
/// fit `budget`, memos of deleted packs or unused for a month, and
/// temporary files a crashed writer left.
fn trim(root: &Path, budget: u64) -> Result<()> {
  let mut packs = Vec::new();
  let mut memos = Vec::new();
  let now = SystemTime::now();
  for entry in std::fs::read_dir(root)?.flatten() {
    let Ok(meta) = entry.metadata() else { continue };
    let modified = meta.modified().unwrap_or(now);
    let path = entry.path();
    match path.extension().and_then(|extension| extension.to_str()) {
      Some("pack" | "ocr") => packs.push((modified, meta.len(), path)),
      Some("id") => memos.push((modified, path)),
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
  for (modified, memo) in memos {
    let bytes = std::fs::read(&memo).unwrap_or_default();
    let alias = bytes.last_chunk::<32>();
    let unused = now.duration_since(modified).unwrap_or_default() > FORGOTTEN;
    if unused || alias.is_none_or(|alias| !path(root, alias).exists()) {
      drop(std::fs::remove_file(memo));
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
    let pack = Pack::open(root.path(), &alias).context("pack")?;
    assert_eq!(hits(&pack.index(PageId(0)).context("page 0")?, "beta"), 1);
    assert!(pack.index(PageId(1)).is_none());
    assert_eq!(pack.pages(), 3);
    assert!(Pack::open(root.path(), &[8; 32]).is_none());
    save(root.path(), &alias, &records(3, &[(2, "gamma")]), Some(&pack))?;
    let pack = Pack::open(root.path(), &alias).context("merged")?;
    assert_eq!(hits(&pack.index(PageId(0)).context("kept")?, "alpha"), 1);
    assert_eq!(hits(&pack.index(PageId(2)).context("added")?, "gamma"), 1);
    let order = [PageId(2), PageId(1), PageId(0)];
    let found = |budget: usize| -> Vec<(usize, usize)> {
      (pack.indexes(&order, budget).iter())
        .map(|(page, index)| {
          (page.0, hits(index, "alpha") + hits(index, "gamma"))
        })
        .collect()
    };
    assert_eq!(found(usize::MAX), [(2, 1), (0, 1)]);
    // A small budget still decodes the first page.
    assert_eq!(found(1), [(2, 1)]);
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
      let pack = Pack::open(root.path(), &alias);
      let read =
        pack.map(|pack| (pack.index(PageId(0)), pack.index(PageId(1))));
      assert!(!matches!(read, Some((Some(_), Some(_)))), "{len}");
    }
    // A decoded length off by one either way.
    let table = MAGIC.len() + 4 + provenance().len() + 4;
    for len in [3, 5] {
      let mut wrong = bytes.clone();
      let at = table + ENTRY + 12;
      let real = u32_at(&wrong, at);
      let changed = (real + len - 4).to_le_bytes();
      wrong[at..at + 4].copy_from_slice(&changed);
      std::fs::write(&file, &wrong)?;
      let pack = Pack::open(root.path(), &alias).context("length")?;
      assert!(pack.index(PageId(0)).is_some(), "{len}");
      assert!(pack.index(PageId(1)).is_none(), "{len}");
    }
    let mut stale = bytes.clone();
    stale[MAGIC.len() + 4] ^= 1;
    std::fs::write(&file, &stale)?;
    assert!(Pack::open(root.path(), &alias).is_none(), "provenance");
    let mut flipped = bytes.clone();
    let last = flipped.len() - 2;
    flipped[last] ^= 0xff;
    std::fs::write(&file, &flipped)?;
    let pack = Pack::open(root.path(), &alias).context("table intact")?;
    assert!(!records(2, &[(1, "two")]).adds_to(Some(&pack)), "held");
    assert!(pack.index(PageId(1)).is_none(), "damaged record");
    // A save replaces the damaged record with the page read again.
    let again = records(2, &[(1, "two")]);
    assert!(again.adds_to(Some(&pack)), "damaged counts as missing");
    save(root.path(), &alias, &again, Some(&pack))?;
    let repaired = Pack::open(root.path(), &alias).context("repaired")?;
    assert_eq!(hits(&repaired.index(PageId(1)).context("page 1")?, "two"), 1);
    assert_eq!(hits(&repaired.index(PageId(0)).context("page 0")?, "one"), 1);
    std::fs::write(&file, &bytes)?;
    let pack = Pack::open(root.path(), &alias).context("restored")?;
    std::fs::remove_file(&file)?;
    assert_eq!(hits(&pack.index(PageId(1)).context("deleted")?, "two"), 1);
    assert!(Pack::open(root.path(), &alias).is_none());
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
    Pack::open(root.path(), &alias).context("pack")?;
    assert!(std::fs::metadata(&file)?.modified()? > old);
    Ok(())
  }

  /// Memos answer only their exact file version, and trimming drops
  /// those unused for long or whose pack is gone.
  #[test]
  fn memos_recall_their_version() -> Result<()> {
    let root = tempfile::tempdir()?;
    let file = root.path().join("doc.pdf");
    std::fs::write(&file, b"%PDF")?;
    let version = Version::read(&file).context("version")?;
    assert_eq!(recall(root.path(), version), None);
    // Temporary directories on macOS lie on APFS.
    assert_eq!(exact(&file), cfg!(target_os = "macos"));
    let alias = [5; 32];
    remember(root.path(), version, &alias)?;
    assert_eq!(recall(root.path(), version), Some(alias));
    // A rewrite of the same size still changes the version.
    std::fs::write(&file, b"%PDX")?;
    let changed = Version::read(&file).context("changed")?;
    assert_ne!(changed, version);
    assert_eq!(recall(root.path(), changed), None);
    save(root.path(), &alias, &records(1, &[(0, "words")]), None)?;
    assert_eq!(recall(root.path(), version), Some(alias), "pack kept");
    let memo = memo(root.path(), version);
    let old = SystemTime::now() - FORGOTTEN * 2;
    File::options().write(true).open(&memo)?.set_modified(old)?;
    trim(root.path(), BUDGET)?;
    assert!(!memo.exists(), "unused");
    remember(root.path(), version, &alias)?;
    std::fs::remove_file(path(root.path(), &alias))?;
    trim(root.path(), BUDGET)?;
    assert_eq!(recall(root.path(), version), None, "pack gone");
    Ok(())
  }

  /// OCR output reads back by its key; other keys and damage miss; it
  /// counts toward the budget.
  #[test]
  fn ocr_round_trips() -> Result<()> {
    let root = tempfile::tempdir()?;
    let key = [6; 32];
    let tsv = "level\tpage\n5\t1\tword\n";
    remember_ocr(root.path(), &key, tsv)?;
    assert_eq!(recall_ocr(root.path(), &key).as_deref(), Some(tsv));
    assert_eq!(recall_ocr(root.path(), &[7; 32]), None);
    let file = ocr_path(root.path(), &key);
    let bytes = std::fs::read(&file)?;
    for len in 0..bytes.len() {
      std::fs::write(&file, &bytes[..len])?;
      assert_eq!(recall_ocr(root.path(), &key), None, "{len}");
    }
    std::fs::write(&file, &bytes)?;
    trim(root.path(), 0)?;
    assert!(!file.exists());
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
    let pack = Pack::open(root.path(), &alias).context("pack")?;
    let page = pack.index(PageId(0)).context("page")?;
    assert_eq!(words.iter().map(|word| hits(&page, word)).sum::<usize>(), 1);
    let files = std::fs::read_dir(root.path())?.count();
    assert_eq!(files, 1, "temporary packs remain");
    Ok(())
  }
}
