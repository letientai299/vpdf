use crate::Result;
use crate::app::App;
use crate::document::{Document, GLYPH_LIMIT, MIB};
use crate::layout::page_matrix;
use crate::text::{Bounds, Glyph, Line, PageId, Text};
use mupdf::{Colorspace, ImageFormat};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

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
      let _ = send.send(result.map(Arc::new).map_err(|e| e.to_string()));
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
  let doc = Document::open(path, password)?;
  doc.validate(page)?;
  let bounds = doc.pages[page.0].bounds;
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
  let mut png = Vec::new();
  pixels.write_to(&mut png, ImageFormat::PNG)?;
  anyhow::ensure!(!cancel.load(Ordering::Relaxed), "OCR cancelled");
  let output = tesseract(&png, language, cancel)?;
  parse_tsv(&output, bounds, scale)
}

fn tesseract(
  png: &[u8],
  language: &str,
  cancel: &AtomicBool,
) -> Result<String> {
  let mut child = Command::new("tesseract")
    .args(["stdin", "stdout", "-l", language, "tsv"])
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
  let bytes = png.to_owned();
  let writer = thread::spawn(move || stdin.write_all(&bytes));
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
      self.reader.cursor.page,
      language,
    ));
    self.reader.message = "OCR running; Esc cancels".into();
    Ok(())
  }

  pub fn ocr_updates(&mut self) -> Result<bool> {
    let Some(job) = &self.ocr else { return Ok(false) };
    let Ok(result) = job.result.try_recv() else { return Ok(false) };
    let page = job.page;
    self.ocr = None;
    match result {
      Ok(text) => {
        if let Some(worker) = &self.search.worker {
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
