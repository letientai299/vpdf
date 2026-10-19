use super::store::{BUDGET, publish, room, trim};
use super::{Alias, RECORD, path, provenance::provenance};
use crate::Result;
use crate::search::Index;
use crate::text::PageId;
use anyhow::{Context, ensure};
use flate2::write::ZlibEncoder;
use flate2::{Compression, Decompress, FlushDecompress, Status};
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

pub(super) const MAGIC: &[u8; 8] = b"VPDFIDX1";

/// Table bytes per page: offset, stored size, decoded size.
pub(super) const ENTRY: usize = 16;

/// Fewest pages a decoder thread takes; smaller batches decode on the
/// calling thread.
const SHARE: usize = 16;

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
    threads: std::num::NonZeroUsize,
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
    let chunk = pages.len().div_ceil(threads.get()).max(SHARE);
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
pub(super) fn inflate(raw: &[u8], len: usize) -> Option<Vec<u8>> {
  // One spare byte lets a longer stream show itself.
  let mut bytes = Vec::with_capacity(len + 1);
  let status = Decompress::new(true)
    .decompress_vec(raw, &mut bytes, FlushDecompress::Finish)
    .ok()?;
  (status == Status::StreamEnd && bytes.len() == len).then_some(bytes)
}

pub(super) fn deflate(bytes: &[u8]) -> Result<Vec<u8>> {
  let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
  encoder.write_all(bytes)?;
  Ok(encoder.finish()?)
}

pub(super) fn u32_at(bytes: &[u8], at: usize) -> u32 {
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

pub(super) fn put(bytes: &mut Vec<u8>, value: usize) {
  bytes.extend_from_slice(&(value as u32).to_le_bytes());
}
