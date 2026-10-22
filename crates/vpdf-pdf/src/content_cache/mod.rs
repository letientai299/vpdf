//! Search records on disk, so reopening a PDF skips its first full scan.
//!
//! One pack per PDF, named by the SHA-256 of its bytes, holds a
//! compressed `Index` record per page. Packs live where the OS may purge
//! them; a missing, short, damaged, or stale pack reads as a miss. Where
//! file versions are exact, a memo per version remembers its hash, so a
//! reopen need not hash. OCR output lives beside the packs, named by a
//! hash of the raster and the engine that read it.
use crate::Result;
use crate::document::MIB;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs::{File, Metadata};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

const OCR_MAGIC: &[u8; 8] = b"VPDFOCR1";
/// Largest decoded page record; a page holds at most `GLYPH_LIMIT`
/// glyphs.
const RECORD: u64 = 16 * MIB as u64;

mod outline;
mod pack;

pub(crate) use outline::OutlineCache;
mod provenance;
mod store;

pub use pack::{Pack, Records, save};
use pack::{deflate, inflate, put};
use store::{BUDGET, create, publish, room, touch, trim};

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
  alias_with(file, || true)
}

pub(super) fn alias_with(
  file: &mut impl Read,
  mut ready: impl FnMut() -> bool,
) -> Result<Alias> {
  let mut hasher = Sha256::new();
  let mut buffer = vec![0; MIB];
  loop {
    anyhow::ensure!(ready(), "PDF hashing stopped");
    match file.read(&mut buffer)? {
      0 => return Ok(hasher.finalize().into()),
      read => hasher.update(&buffer[..read]),
    }
  }
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

fn ocr_path(root: &Path, key: &Alias) -> PathBuf {
  root.join(format!("{}.ocr", hex(key)))
}

#[cfg(test)]
mod tests {
  use super::pack::{ENTRY, MAGIC, u32_at};
  use super::provenance::provenance;
  use super::store::{ABANDONED, FORGOTTEN};
  use super::*;
  use crate::search::Index;
  use crate::search::compile;
  use crate::text::PageId;
  use crate::text::{Bounds, Glyph, Line, Text};
  use anyhow::Context;
  use std::time::{Duration, SystemTime};

  fn index(page: usize, words: &str) -> Index {
    let line = Line {
      paragraph: true,
      glyphs: words
        .chars()
        .map(|ch| Glyph { quad: None, ch, bounds: Bounds::default() })
        .collect(),
    };
    Index::new(PageId(page), &Text { lines: vec![line], truncated: false })
  }

  #[test]
  fn hashing_stops_between_chunks() -> Result<()> {
    let bytes = vec![7; 3 * MIB];
    for allowed in 0..=1 {
      let mut file = std::io::Cursor::new(&bytes);
      let mut checks = 0;
      assert!(
        alias_with(&mut file, || {
          checks += 1;
          checks <= allowed
        })
        .is_err()
      );
      assert_eq!(file.position(), (allowed * MIB) as u64);
    }
    let mut file = std::io::Cursor::new(&bytes);
    let expected: Alias = Sha256::digest(&bytes).into();
    assert_eq!(alias_with(&mut file, || true)?, expected);
    Ok(())
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
      (pack
        .indexes(&order, budget, crate::jobs::Config::default().helpers)
        .iter())
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
