mod skew;
use super::{OneShot, Outcome, Wake};
use crate::Result;
use crate::content_cache::{self, Alias, Version};
use crate::document::{GLYPH_LIMIT, Handle, MIB};
use crate::text::{Bounds, Glyph, Line, PageId, Text};
use mupdf::{Colorspace, ImageFormat};
use sha2::Digest;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use vpdf_model::geometry::page_matrix;

/// Tesseract's output arguments, part of every stored result's key.
const OUTPUT: [&str; 3] = ["stdin", "stdout", "tsv"];
/// The header every TSV output starts with.
const TSV: &str = "level\tpage_num\t";

#[derive(Clone, Copy)]
enum Request {
  Explicit,
  Automatic,
}

pub struct OcrJob {
  request: Request,
  pub page: PageId,
  pub work: OneShot<Outcome<Option<Arc<Text>>>>,
}

impl OcrJob {
  pub fn start(
    path: &Path,
    password: &str,
    page: PageId,
    language: &str,
    wake: &Wake,
  ) -> Self {
    Self::spawn(path, password, page, language, wake, Request::Explicit)
  }

  pub fn automatic(
    path: &Path,
    password: &str,
    page: PageId,
    language: &str,
    wake: &Wake,
  ) -> Self {
    Self::spawn(path, password, page, language, wake, Request::Automatic)
  }

  pub fn is_automatic(&self) -> bool {
    matches!(self.request, Request::Automatic)
  }

  fn spawn(
    path: &Path,
    password: &str,
    page: PageId,
    language: &str,
    wake: &Wake,
    request: Request,
  ) -> Self {
    let path = path.to_owned();
    let password = password.to_owned();
    let language = language.to_owned();
    let work = OneShot::try_spawn(wake, move |stop| {
      let result = match request {
        Request::Explicit => {
          recognize(&path, &password, page, &language, stop).map(Some)
        }
        Request::Automatic => {
          automatic(&path, &password, page, &language, stop)
        }
      };
      Some(result.map(|text| text.map(Arc::new)))
    });
    Self { request, page, work }
  }

  pub fn cancel(&self) {
    self.work.cancel();
  }
}

fn recognize(
  path: &Path,
  password: &str,
  page: PageId,
  language: &str,
  cancel: &AtomicBool,
) -> Result<Text> {
  anyhow::ensure!(
    language
      .chars()
      .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '+' | '-')),
    "Invalid OCR language"
  );
  let doc = Handle::open(path, password)?;
  recognize_page(&doc, password, page, language, cancel)
}

fn ensure_live(cancel: &AtomicBool) -> Result<()> {
  anyhow::ensure!(crate::scheduling::ready(cancel), "OCR cancelled");
  Ok(())
}

fn automatic(
  path: &Path,
  password: &str,
  page: PageId,
  language: &str,
  cancel: &AtomicBool,
) -> Result<Option<Text>> {
  static SLOT: std::sync::Mutex<()> = std::sync::Mutex::new(());
  let _slot = loop {
    ensure_live(cancel)?;
    match SLOT.try_lock() {
      Ok(slot) => break slot,
      Err(std::sync::TryLockError::Poisoned(_)) => {
        anyhow::bail!("OCR worker stopped")
      }
      Err(std::sync::TryLockError::WouldBlock) => {}
    }
    thread::sleep(Duration::from_millis(20));
  };
  ensure_live(cancel)?;
  let doc = Handle::open(path, password)?;
  ensure_live(cancel)?;
  if !doc.text(page)?.lines.is_empty() {
    return Ok(None);
  }
  ensure_live(cancel)?;
  let bounds = doc.bounds(page)?;
  let raw = doc.native.load_page(page.0 as i32)?.to_text_page(
    crate::document::TEXT_FLAGS | mupdf::TextPageFlags::PRESERVE_IMAGES,
  )?;
  let scanned = raw.blocks().any(|block| {
    let b = crate::document::rect(block.bounds());
    matches!(block.r#type(), mupdf::text_page::TextBlockType::Image)
      && b.width() * b.height() >= bounds.width() * bounds.height() * 0.25
  });
  if !scanned || engine(language).is_none() {
    return Ok(None);
  }
  ensure_live(cancel)?;
  recognize_page(&doc, password, page, language, cancel).map(Some)
}

fn recognize_page(
  doc: &Handle,
  password: &str,
  page: PageId,
  language: &str,
  cancel: &AtomicBool,
) -> Result<Text> {
  ensure_live(cancel)?;
  let bounds = doc.bounds(page)?;
  let scale =
    2.5_f32.min((16_000_000.0 / (bounds.width() * bounds.height())).sqrt());
  let scale_key = (scale * 1024.0) as u32;
  let scale = scale_key as f32 / 1024.0;
  let pixels = doc.native.load_page(page.0 as i32)?.to_pixmap(
    &crate::raster::matrix(&page_matrix(bounds, scale_key, 0)),
    &Colorspace::device_rgb(),
    false,
    true,
  )?;
  // Decrypted text stays off disk.
  let root = content_cache::root().filter(|_| password.is_empty());
  let key = root.as_ref().and_then(|_| {
    let rows = Rows {
      width: pixels.width(),
      height: pixels.height(),
      n: pixels.n(),
      stride: usize::try_from(pixels.stride()).ok()?,
    };
    Some(raster_key(&engine(language)?, language, rows, pixels.samples()))
  });
  ensure_live(cancel)?;
  let output = cached(root.as_deref(), key.as_ref(), || {
    let mut png = Vec::new();
    pixels.write_to(&mut png, ImageFormat::PNG)?;
    anyhow::ensure!(!cancel.load(Ordering::Relaxed), "OCR cancelled");
    tesseract(png, language, cancel)
  })?;
  anyhow::ensure!(!cancel.load(Ordering::Relaxed), "OCR cancelled");
  parse_tsv(&output, bounds, scale)
}

/// OCR output stored under `key`, else `run`'s, stored for next time.
fn cached(
  root: Option<&Path>,
  key: Option<&Alias>,
  run: impl FnOnce() -> Result<String>,
) -> Result<String> {
  let (Some(root), Some(key)) = (root, key) else { return run() };
  if let Some(output) = content_cache::recall_ocr(root, key) {
    return Ok(output);
  }
  let output = run()?;
  // Tesseract without its tsv config prints plain text and succeeds;
  // only real TSV is worth keeping.
  if output.starts_with(TSV) {
    // A failed store only costs the next run.
    drop(content_cache::remember_ocr(root, key, &output));
  }
  Ok(output)
}

/// A raster's layout: rows of `width` pixels of `n` bytes, `stride`
/// bytes apart.
#[derive(Clone, Copy)]
struct Rows {
  width: u32,
  height: u32,
  n: u8,
  stride: usize,
}

/// What OCR reads: the engine, the language, and the raster's pixels,
/// without row padding.
fn raster_key(
  engine: &str,
  language: &str,
  rows: Rows,
  samples: &[u8],
) -> Alias {
  let output = OUTPUT.join(" ");
  let mut hasher = content_cache::digest([
    b"ocr 1",
    engine.as_bytes(),
    language.as_bytes(),
    output.as_bytes(),
  ]);
  for field in [rows.width, rows.height, u32::from(rows.n)] {
    hasher.update(field.to_le_bytes());
  }
  let row = rows.width as usize * usize::from(rows.n);
  for line in samples.chunks(rows.stride.max(1)).take(rows.height as usize) {
    hasher.update(line.get(..row).unwrap_or(line));
  }
  hasher.finalize().into()
}

/// Tesseract's version, its tsv config, and each model `language` names,
/// by file version; none when any cannot be read.
fn engine(language: &str) -> Option<String> {
  // Asked once: each answer costs a process.
  static INSTALL: OnceLock<Option<(String, PathBuf)>> = OnceLock::new();
  let (version, models) = INSTALL
    .get_or_init(|| {
      let output = |arg: &str| {
        let output = Command::new("tesseract").arg(arg).output().ok()?;
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
      };
      let models = tessdata(&output("--list-langs")?)?.to_owned();
      Some((output("--version")?, models))
    })
    .as_ref()?;
  let mut engine = version.clone();
  let config = models.join("configs/tsv");
  let _ = write!(engine, "\ntsv {:?}", Version::read(&config)?);
  for name in language.split('+') {
    let model = models.join(format!("{name}.traineddata"));
    let _ = write!(engine, "\n{name} {:?}", Version::read(&model)?);
  }
  Some(engine)
}

/// The model directory a `tesseract --list-langs` listing names.
fn tessdata(listing: &str) -> Option<&Path> {
  let line = listing.lines().next()?;
  line.starts_with("List of available languages in ").then_some(())?;
  Some(Path::new(line.split('"').nth(1)?))
}

pub(crate) fn tesseract(
  png: Vec<u8>,
  language: &str,
  cancel: &AtomicBool,
) -> Result<String> {
  let mut child = Command::new("tesseract")
    .env("OMP_THREAD_LIMIT", "1")
    .args(["-l", language])
    .args(OUTPUT)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;
  let stdout =
    child.stdout.take().ok_or_else(|| anyhow::anyhow!("OCR stdout missing"))?;
  let stderr =
    child.stderr.take().ok_or_else(|| anyhow::anyhow!("OCR stderr missing"))?;
  let reader = thread::spawn(move || bounded_read(stdout, 8 * MIB));
  let errors = thread::spawn(move || bounded_read(stderr, 64 * 1024));
  let mut stdin =
    child.stdin.take().ok_or_else(|| anyhow::anyhow!("OCR stdin missing"))?;
  let writer = thread::spawn(move || stdin.write_all(&png));
  let started = Instant::now();
  let status = loop {
    if cancel.load(Ordering::Relaxed) || started.elapsed().as_secs() > 120 {
      let _ = child.kill();
      let _ = child.wait();
      let _ = writer.join();
      let _ = reader.join();
      let _ = errors.join();
      anyhow::bail!("OCR cancelled or timed out");
    }
    if let Some(status) = child.try_wait()? {
      break status;
    }
    thread::sleep(Duration::from_millis(20));
  };
  let _ = writer.join();
  let output =
    reader.join().map_err(|_| anyhow::anyhow!("OCR reader failed"))??;
  let errors =
    errors.join().map_err(|_| anyhow::anyhow!("OCR diagnostics failed"))??;
  anyhow::ensure!(status.success(), "OCR failed: {}", errors.trim());
  Ok(output)
}

fn bounded_read(mut input: impl Read, limit: usize) -> Result<String> {
  let mut bytes = Vec::new();
  input.by_ref().take(limit as u64).read_to_end(&mut bytes)?;
  std::io::copy(&mut input, &mut std::io::sink())?;
  Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn parse_tsv(tsv: &str, bounds: Bounds, scale: f32) -> Result<Text> {
  anyhow::ensure!(scale.is_finite() && scale > 0.0, "Invalid OCR scale");
  let mut text = Text::default();
  let mut words = Vec::new();
  let mut previous = String::new();
  let mut previous_paragraph = String::new();
  let mut count = 0;
  'records: for record in tsv.lines().skip(1) {
    let fields: Vec<_> = record.splitn(12, '\t').collect();
    if fields.len() != 12 || fields[0] != "5" || fields[11].trim().is_empty() {
      continue;
    }
    let id = fields[1..5].join(":");
    if id != previous {
      let paragraph = fields[1..4].join(":");
      text.lines.push(Line {
        ocr: None,
        paragraph: paragraph != previous_paragraph,
        ..Line::default()
      });
      words.push(Vec::new());
      previous_paragraph = paragraph;
      previous = id;
    }
    let x = fields[6].parse::<f32>()? / scale + bounds.x0;
    let y = fields[7].parse::<f32>()? / scale + bounds.y0;
    let width = fields[8].parse::<f32>()? / scale;
    let height = fields[9].parse::<f32>()? / scale;
    anyhow::ensure!(
      [x, y, width, height, x + width, y + height]
        .into_iter()
        .all(f32::is_finite)
        && width >= 0.0
        && height >= 0.0,
      "Invalid OCR geometry"
    );
    let Some(row_words) = words.last_mut() else { continue };
    row_words.push(Bounds { x0: x, y0: y, x1: x + width, y1: y + height });
    let chars: Vec<_> =
      fields[11].chars().filter(|c| !c.is_control()).collect();
    let Some(line) = text.lines.last_mut() else { continue };
    if let Some(last) = line.glyphs.last() {
      line.glyphs.push(Glyph {
        quad: None,
        ch: ' ',
        bounds: Bounds { x0: last.bounds.x1, y0: y, x1: x, y1: y + height },
      });
    }
    for (index, ch) in chars.iter().enumerate() {
      if count >= GLYPH_LIMIT {
        text.truncated = true;
        break 'records;
      }
      let step = width / chars.len() as f32;
      line.glyphs.push(Glyph {
        quad: None,
        ch: *ch,
        bounds: Bounds {
          x0: x + step * index as f32,
          y0: y,
          x1: x + step * (index + 1) as f32,
          y1: y + height,
        },
      });
      count += 1;
    }
  }
  skew::apply(&mut text, &words, bounds);
  Ok(text)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::cell::Cell;
  use std::os::unix::fs::MetadataExt;
  use std::sync::mpsc;

  #[test]
  #[ignore = "requires corpus and Tesseract"]
  fn real_ocr_cache_policy() -> Result<()> {
    if std::env::var_os("VPDF_OCR_CACHE_CHILD").is_none() {
      let cache = tempfile::tempdir()?;
      let status = Command::new(std::env::current_exe()?)
        .args(["--exact", "jobs::ocr::tests::real_ocr_cache_policy"])
        .args(["--ignored", "--nocapture"])
        .env("VPDF_OCR_CACHE_CHILD", "1")
        .env("VPDF_CACHE", cache.path())
        .status()?;
      anyhow::ensure!(status.success(), "OCR cache test failed");
      return Ok(());
    }
    let path = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("ocr.pdf");
    let hash =
      |path: &Path| content_cache::alias(&mut std::fs::File::open(path)?);
    let original = hash(&path)?;
    let root = content_cache::root().expect("cache enabled");
    let cancel = AtomicBool::new(false);
    let expected = recognize(&path, "", PageId(0), "eng", &cancel)?.plain();
    assert!(expected.to_lowercase().contains("cheese"));
    let files =
      std::fs::read_dir(&root)?.collect::<std::io::Result<Vec<_>>>()?;
    assert_eq!(files.len(), 1, "unencrypted OCR writes its cache");
    assert_eq!(files[0].path().extension().unwrap(), "ocr");
    assert!(
      automatic(&path, "", PageId(0), "eng", &cancel)?.is_none(),
      "embedded OCR is left alone"
    );
    std::fs::remove_file(files[0].path())?;

    let mut doc = Handle::open(&path, "")?;
    let file = crate::encryption::encrypt(&mut doc.native)?;
    let before = hash(file.path())?;
    assert!(Handle::open(file.path(), "").is_err());
    let text =
      recognize(file.path(), "test-reader", PageId(0), "eng", &cancel)?;
    assert_eq!(text.plain(), expected);
    assert_eq!(std::fs::read_dir(&root)?.count(), 0);
    assert_eq!(hash(&path)?, original);
    assert_eq!(hash(file.path())?, before);
    let scan = path.with_file_name("scan.pdf");
    let first = automatic(&scan, "", PageId(9), "eng", &cancel)?
      .expect("scan recognized")
      .plain();
    assert!(first.contains("computing devices"));
    assert!(
      automatic(&scan, "", PageId(9), "vpdf_missing_language", &cancel)?
        .is_none()
    );
    let files =
      std::fs::read_dir(&root)?.collect::<std::io::Result<Vec<_>>>()?;
    assert_eq!(files.len(), 1);
    let stored = std::fs::read(files[0].path())?;
    let inode = std::fs::metadata(files[0].path())?.ino();
    assert_eq!(
      automatic(&scan, "", PageId(9), "eng", &cancel)?.unwrap().plain(),
      first
    );
    assert_eq!(
      std::fs::read(files[0].path())?,
      stored,
      "warm automatic OCR keeps its disk record"
    );
    assert_eq!(std::fs::metadata(files[0].path())?.ino(), inode);

    Ok(())
  }

  #[test]
  fn automatic_yields_before_opening() -> Result<()> {
    let urgent = crate::scheduling::Urgent::new();
    let job = OcrJob::automatic(
      Path::new("/nonexistent/scan.pdf"),
      "",
      PageId(0),
      "eng",
      &Wake::none(),
    );
    assert!(matches!(
      job.work.result.recv_timeout(Duration::from_millis(30)),
      Err(mpsc::RecvTimeoutError::Timeout)
    ));
    job.cancel();
    let result = job.work.result.recv_timeout(Duration::from_secs(5))?;
    assert!(result.unwrap_err().contains("OCR cancelled"));
    drop(urgent);
    Ok(())
  }

  #[test]
  fn geometry_rejects_nonfinite_records() {
    for value in ["NaN", "inf", "-1"] {
      let tsv =
        format!("header\n5\t1\t1\t1\t1\t1\t0\t0\t{value}\t10\t99\tword\n");
      assert!(parse_tsv(&tsv, Bounds::default(), 1.0).is_err());
    }
  }

  /// Stored output answers the same key without running the engine;
  /// without a cache, the engine always runs.
  #[test]
  fn stored_output_skips_the_engine() -> Result<()> {
    let root = tempfile::tempdir()?;
    let runs = Cell::new(0);
    let run = || {
      runs.set(runs.get() + 1);
      Ok(format!("{TSV}{}", runs.get()))
    };
    let key = [3; 32];
    let tsv = |run: usize| format!("{TSV}{run}");
    assert_eq!(cached(Some(root.path()), Some(&key), run)?, tsv(1));
    assert_eq!(cached(Some(root.path()), Some(&key), run)?, tsv(1));
    assert_eq!(cached(Some(root.path()), Some(&[4; 32]), run)?, tsv(2));
    assert_eq!(cached(None, Some(&key), run)?, tsv(3));
    assert_eq!(cached(Some(root.path()), None, run)?, tsv(4));
    // Failures and plain text are returned, never kept.
    let other = [5; 32];
    assert!(
      cached(Some(root.path()), Some(&other), || anyhow::bail!("no")).is_err()
    );
    let plain = || Ok("plain words".to_owned());
    assert_eq!(cached(Some(root.path()), Some(&other), plain)?, "plain words");
    assert_eq!(cached(Some(root.path()), Some(&other), run)?, tsv(5));
    Ok(())
  }

  /// Listings name their model directory in Tesseract 5's header.
  #[test]
  fn listings_name_their_models() {
    let cases = [
      (
        "List of available languages in \"/opt/tessdata/\" (2):\neng\nosd\n",
        Some("/opt/tessdata/"),
      ),
      ("List of available languages (2):\neng\n", None),
      ("eng\nosd\n", None),
      ("", None),
    ];
    for (listing, models) in cases {
      assert_eq!(tessdata(listing), models.map(Path::new), "{listing}");
    }
  }

  /// Keys ignore row padding and change with pixels, language, engine.
  #[test]
  fn raster_keys_follow_what_ocr_reads() {
    let rows = Rows { width: 2, height: 2, n: 3, stride: 6 };
    let pixels = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    let padded = [1, 2, 3, 4, 5, 6, 0, 0, 7, 8, 9, 10, 11, 12, 0, 0];
    let key = raster_key("5.5", "eng", rows, &pixels);
    let wide = Rows { stride: 8, ..rows };
    assert_eq!(raster_key("5.5", "eng", wide, &padded), key);
    let mut changed = pixels;
    changed[11] = 0;
    for other in [
      raster_key("5.5", "eng", rows, &changed),
      raster_key("5.5", "deu", rows, &pixels),
      raster_key("5.4", "eng", rows, &pixels),
      raster_key("5.5", "eng", Rows { width: 1, ..wide }, &padded),
    ] {
      assert_ne!(other, key);
    }
  }
}
