use crate::Result;
use crate::app::App;
use crate::document::Target;
use crate::text::{Bounds, PageId};
use crossterm::event::KeyEvent;
use std::io::Write;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::hints::{Hints, Transition};
use vpdf_ui::paint::Frame;

impl App {
  /// Labels links in view; `copy` copies URIs instead of opening them.
  pub fn start_hints(&mut self, copy: bool) -> Result<()> {
    let mut found = Vec::new();
    let pages = if self.text_mode {
      self.text().pages()?
    } else {
      self.layout.visible(self.reader.scroll)
    };
    for page in pages {
      for link in self.doc.links(PageId(page))?.iter() {
        if let Some(spot) = vpdf_ui::hints::spot(link, |bounds| {
          self.link_cell(PageId(page), bounds)
        })? {
          found.push((spot, link.target.clone()));
        }
      }
    }
    self.hints = Some(Hints::new(found, copy, self.left, self.screen.columns)?);
    Ok(())
  }

  fn link_cell(
    &mut self,
    page: PageId,
    bounds: Bounds,
  ) -> Result<Option<(u16, u16)>> {
    // Padded boxes reach the line above.
    let middle = f32::midpoint(bounds.y0, bounds.y1);
    if self.text_mode {
      let position = self.doc.text(page)?.nearest(page, bounds.x0, middle);
      return self.text().cell(position);
    }
    vpdf_ui::hints::page_cell(
      &self.layout,
      self.reader.scroll,
      self.left,
      page,
      bounds,
    )
  }

  pub fn hint_key(&mut self, key: KeyEvent) -> Result<()> {
    let Some(hints) = &mut self.hints else { return Ok(()) };
    match hints.key(key) {
      Transition::Stay => Ok(()),
      Transition::Run(index) => self.run_hint(index),
      Transition::Cancel => {
        self.hints = None;
        self.reader.cancel();
        self.disarm();
        Ok(())
      }
      Transition::Close => {
        self.hints = None;
        Ok(())
      }
      Transition::Miss(typed) => {
        self.hints = None;
        anyhow::bail!("No link labeled {typed}")
      }
    }
  }

  pub(crate) fn run_hint(&mut self, index: usize) -> Result<()> {
    let Some(hints) = self.hints.take() else { return Ok(()) };
    let Some((target, copy)) = hints.into_target(index) else { return Ok(()) };
    match target {
      Target::Uri(uri) if copy => self.copy_address(uri),
      target => self.follow(&target),
    }
  }

  pub fn draw_hints(
    &mut self,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<()> {
    let Some(hints) = &self.hints else { return Ok(()) };
    hints.draw(
      graphics,
      out,
      Frame {
        at: self.at,
        screen: self.layout.screen(),
        left: self.left,
        text: self.text_mode,
      },
    )
  }
}
