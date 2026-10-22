use crate::state;
use vpdf_pdf::jobs::{Wake, recent::Worker};
use vpdf_ui::panel::recent::{Entry, Shelf};

pub struct Loader {
  worker: Worker,
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
    Self { worker: Worker::new(password, wake) }
  }

  pub fn request(&self, cards: &mut Shelf) {
    if let Some(request) = cards.take_request() {
      self.worker.request(request);
    }
  }

  pub fn updates(&self, cards: &mut Shelf) -> bool {
    let mut arrived = false;
    for (path, loaded) in self.worker.updates() {
      cards.accept(path, loaded);
      arrived = true;
    }
    arrived
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::Result;
  use anyhow::Context;
  use std::path::PathBuf;
  use std::time::{Duration, Instant};
  use vpdf_model::geometry::At;
  use vpdf_terminal::graphics::Graphics;
  use vpdf_ui::{
    keys::PanelKind, layout::Screen, panel::Panel, panel::recent::Frame,
  };

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
    let screen = Screen {
      columns: 100,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
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
    woken.recv_timeout(Duration::from_secs(10))?;
    let (path, card) =
      loader.worker.updates().next().context("Card never arrived")?;
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
    let screen = Screen {
      columns: 100,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
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
    while !loader.updates(&mut cards) {
      anyhow::ensure!(Instant::now() < deadline, "Card never loaded");
      std::thread::sleep(Duration::from_millis(5));
    }
    let frame = draw(&mut cards)?;
    for want in ["Page", "of", "Parent", "Pages", "Size", "Metadata", "a=t,"] {
      assert!(frame.contains(want), "{want}");
    }
    // A drawn card asks nothing more.
    draw(&mut cards)?;
    std::thread::sleep(Duration::from_millis(200));
    assert!(loader.worker.updates().next().is_none());
    Ok(())
  }
}
