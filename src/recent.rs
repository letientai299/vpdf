use crate::state;
use vpdf_pdf::jobs::{Wake, recent::Worker};
use vpdf_ui::panel::recent::{Entry, Shelf};

pub struct Loader {
  worker: Worker,
  logo: std::sync::mpsc::Receiver<vpdf_model::image::Rgb>,
}

pub fn entries(saved: &[state::Entry]) -> Vec<Entry> {
  saved
    .iter()
    .map(|entry| Entry {
      path: entry.path.clone(),
      cursor: entry.cursor,
      rotation: entry.rotation,
      used: entry.used,
      marks: entry.marks.len(),
    })
    .collect()
}

impl Loader {
  pub fn new(password: &str, wake: &Wake) -> Self {
    let (send, logo) = std::sync::mpsc::channel();
    let send = wake.sender(send);
    std::thread::spawn(move || {
      static LOGO: std::sync::OnceLock<Option<vpdf_model::image::Rgb>> =
        std::sync::OnceLock::new();
      if let Some(logo) = LOGO.get_or_init(|| load_logo().ok()) {
        let _ = send.send(logo.clone());
      }
    });
    Self { worker: Worker::new(password, wake), logo }
  }

  pub fn request(&self, cards: &mut Shelf) {
    if let Some(request) = cards.take_request() {
      self.worker.request(request);
    }
    if let Some(request) = cards.take_counts() {
      self.worker.visible(request);
    }
  }

  pub fn updates(&self, cards: &mut Shelf) -> bool {
    let mut arrived = if let Ok(logo) = self.logo.try_recv() {
      cards.set_logo(logo);
      true
    } else {
      false
    };
    for (path, loaded) in self.worker.updates() {
      cards.accept(path, loaded);
      arrived = true;
    }
    for (generation, path, pages, changed) in self.worker.counts() {
      if changed {
        cards.invalidate(generation, &path);
      }
      cards.accept_count(generation, path, pages);
      arrived = true;
    }
    arrived
  }
}

fn load_logo() -> crate::Result<vpdf_model::image::Rgb> {
  let image =
    mupdf::Image::from_bytes(include_bytes!("../docs/assets/vpdf.png"))?;
  let mut pixmap = image.to_pixmap()?;
  pixmap.shrink(1)?;
  anyhow::ensure!(
    pixmap.n() == 3 || pixmap.n() == 4,
    "Logo colorspace unsupported"
  );
  let mut pixels = pixmap.samples().to_vec();
  let image = if pixmap.alpha() {
    for pixel in pixels.as_chunks_mut::<4>().0 {
      let alpha = u16::from(pixel[3]);
      for color in &mut pixel[..3] {
        *color =
          (u16::from(*color) * 255).checked_div(alpha).unwrap_or(0).min(255)
            as u8;
      }
    }
    vpdf_model::image::Rgb::rgba(
      pixmap.width() as i32,
      pixmap.height() as i32,
      pixels,
    )
  } else {
    vpdf_model::image::Rgb::new(
      pixmap.width() as i32,
      pixmap.height() as i32,
      pixels,
    )
  };
  image.ok_or_else(|| anyhow::anyhow!("Logo pixels invalid"))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::Result;
  use std::path::PathBuf;
  use std::time::{Duration, Instant};
  use vpdf_model::geometry::At;
  use vpdf_terminal::graphics::Graphics;
  use vpdf_ui::{keys::PanelKind, panel::Panel, panel::recent::Frame};

  #[test]
  fn cards_wake_the_loop() -> Result<()> {
    let file = tempfile::NamedTempFile::new()?;
    crate::fixtures::pdf(file.path(), &["body"], "")?;
    let entry = state::tests::entry(&file.path().to_string_lossy(), 0, 0);
    let saved = std::slice::from_ref(&entry);
    let (wake, woken) = crate::fixtures::wake();
    let loader = Loader::new("", &wake);
    let mut cards = Shelf::new(&entries(saved));
    let mut panel =
      Panel::new(PanelKind::Recent, crate::panel::recent_rows(saved));
    let screen = crate::screen::cells(100, 40);
    crate::presentation::draw(
      &mut Vec::new(),
      Some(&mut Graphics::default()),
      |scene| {
        cards.scene(
          &mut panel,
          scene,
          Frame { at: At::default(), width: 100, screen, now: 0 },
          true,
        )
      },
    )??;
    loader.request(&mut cards);
    let deadline = Instant::now() + Duration::from_secs(10);
    let (path, card) = loop {
      if let Some(card) = loader.worker.updates().next() {
        break card;
      }
      woken.recv_timeout(deadline.saturating_duration_since(Instant::now()))?;
    };
    assert_eq!(path, entry.path);
    let facts = card.facts.map_err(anyhow::Error::msg)?;
    assert_eq!(facts.pages, 1);
    assert!(facts.thumb.is_some());
    assert_eq!(woken.try_iter().count(), 0);
    Ok(())
  }
  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn cards_show_facts_and_the_first_page() -> Result<()> {
    let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    let file = root.join("paper.pdf");
    let entry = state::tests::entry(&file.to_string_lossy(), 1, 0);
    let entries = std::slice::from_ref(&entry);
    let screen = crate::screen::cells(100, 40);
    let mut cards = Shelf::new(&super::entries(entries));
    let loader = Loader::new("", &Wake::none());
    let mut panel =
      Panel::new(PanelKind::Recent, crate::panel::recent_rows(entries));
    let mut graphics = Graphics::default();
    let mut draw = |cards: &mut Shelf| -> Result<String> {
      let mut out = Vec::new();
      crate::presentation::draw(&mut out, Some(&mut graphics), |scene| {
        cards.scene(
          &mut panel,
          scene,
          Frame { at: At::default(), width: 100, screen, now: 0 },
          true,
        )
      })??;
      loader.request(cards);
      Ok(String::from_utf8(out)?)
    };
    assert!(draw(&mut cards)?.contains("Loading…"));
    let deadline = Instant::now() + Duration::from_secs(10);
    let frame = loop {
      loader.updates(&mut cards);
      let frame = draw(&mut cards)?;
      if frame.contains("Size") {
        break frame;
      }
      anyhow::ensure!(Instant::now() < deadline, "Card never loaded");
      std::thread::sleep(Duration::from_millis(5));
    };
    for want in ["Page", "Size", "a=t,"] {
      assert!(frame.contains(want), "{want}");
    }
    // A drawn card asks nothing more.
    draw(&mut cards)?;
    std::thread::sleep(Duration::from_millis(200));
    assert!(loader.worker.updates().next().is_none());
    Ok(())
  }
}
