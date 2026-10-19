use crate::Result;
use crate::app::App;
use crate::content_cache::{self, Alias, Version};
use crate::document::{GLYPH_LIMIT, Handle, MIB};
use crate::layout::page_matrix;
use crate::text::{Bounds, Glyph, Line, PageId, Text};
use mupdf::{Colorspace, ImageFormat};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// Tesseract's output arguments, part of every stored result's key.
const OUTPUT: [&str; 3] = ["stdin", "stdout", "tsv"];
/// The header every TSV output starts with.
const TSV: &str = "level\tpage_num\t";

pub struct OcrJob {
  pub page: PageId,
  cancel: Arc<AtomicBool>,
  pub result: mpsc::Receiver<std::result::Result<Arc<Text>, String>>,
}

impl OcrJob {
  pub fn start(
    path: &Path,
    password: &str,
    page: PageId,
    language: &str,
  ) -> Self {
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&cancel);
    let (send, result) = mpsc::channel();
    let path = path.to_owned();
    let password = password.to_owned();
    let language = language.to_owned();
    thread::spawn(move || {
      let result = recognize(&path, &password, page, &language, &stop);
      let result = result.map(Arc::new).map_err(|e| e.to_string());
      let _ = crate::input::notify(send.send(result));
    });
    Self { page, cancel, result }
  }

  pub fn cancel(&self) {
    self.cancel.store(true, Ordering::Relaxed);
  }
}

impl Drop for OcrJob {
  fn drop(&mut self) {
    self.cancel();
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
  let bounds = doc.bounds(page)?;
  let scale =
    2.5_f32.min((16_000_000.0 / (bounds.width() * bounds.height())).sqrt());
  let scale_key = (scale * 1024.0) as u32;
  let scale = scale_key as f32 / 1024.0;
  let pixels = doc.native.load_page(page.0 as i32)?.to_pixmap(
    &page_matrix(bounds, scale_key, 0),
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
  let mut hasher = Sha256::new();
  let output = OUTPUT.join(" ");
  for part in
    [b"ocr 1", engine.as_bytes(), language.as_bytes(), output.as_bytes()]
  {
    hasher.update((part.len() as u64).to_le_bytes());
    hasher.update(part);
  }
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
  let mut previous = String::new();
  let mut previous_paragraph = String::new();
  let mut count = 0;
  for record in tsv.lines().skip(1) {
    let fields: Vec<_> = record.splitn(12, '\t').collect();
    if fields.len() != 12 || fields[0] != "5" || fields[11].trim().is_empty() {
      continue;
    }
    let id = fields[1..5].join(":");
    if id != previous {
      let paragraph = fields[1..4].join(":");
      text.lines.push(Line {
        paragraph: paragraph != previous_paragraph,
        ..Line::default()
      });
      previous_paragraph = paragraph;
      previous = id;
    }
    let x = fields[6].parse::<f32>()? / scale + bounds.x0;
    let y = fields[7].parse::<f32>()? / scale + bounds.y0;
    let width = fields[8].parse::<f32>()? / scale;
    let height = fields[9].parse::<f32>()? / scale;
    let chars: Vec<_> =
      fields[11].chars().filter(|c| !c.is_control()).collect();
    let Some(line) = text.lines.last_mut() else { continue };
    if let Some(last) = line.glyphs.last() {
      line.glyphs.push(Glyph {
        ch: ' ',
        bounds: Bounds { x0: last.bounds.x1, y0: y, x1: x, y1: y + height },
      });
    }
    for (index, ch) in chars.iter().enumerate() {
      if count >= GLYPH_LIMIT {
        text.truncated = true;
        return Ok(text);
      }
      let step = width / chars.len() as f32;
      line.glyphs.push(Glyph {
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
  Ok(text)
}

impl App {
  pub fn start_ocr(&mut self, language: &str) -> Result<()> {
    if let Some(job) = self.ocr.take() {
      job.cancel();
    }
    self.ocr = Some(OcrJob::start(
      &self.path,
      &self.password,
      self.navigation_position().page,
      language,
    ));
    self.reader.message = "OCR running; :cancel stops".into();
    Ok(())
  }

  pub fn ocr_updates(&mut self) -> Result<bool> {
    let Some(job) = &self.ocr else { return Ok(false) };
    let Ok(result) = job.result.try_recv() else { return Ok(false) };
    let page = job.page;
    self.ocr = None;
    match result {
      Ok(text) => {
        if let Some(worker) = &self.search_worker {
          worker.seed(page, Arc::clone(&text));
        }
        self.doc.seed(page, text);
        self.reader.message = "OCR ready".into();
        self.reader.cursor =
          self.doc.text(self.reader.cursor.page)?.clamp(self.reader.cursor);
      }
      Err(error) => self.error = Some(error),
    }
    Ok(true)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::cell::Cell;

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
