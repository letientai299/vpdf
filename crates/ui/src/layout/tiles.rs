use super::{Layout, Screen, visible};
#[cfg(test)]
use crate::presentation::*;
use crate::text::PageId;
#[cfg(test)]
use std::io::Write;
use vpdf_model::scene::Surface;
use vpdf_model::tile::{TileId, TileKey, TileOwner};
#[cfg(test)]
use vpdf_terminal::graphics::Graphics;

pub struct Tiles {
  wanted: Vec<TileKey>,
  heading: f32,
}

#[derive(Clone, Copy)]
pub struct View<'a> {
  pub layout: &'a Layout,
  pub scroll: f32,
  pub target: Option<f32>,
}

#[derive(Clone, Copy)]
pub struct Frame {
  pub pane: u32,
  pub screen: Screen,
  pub left: u16,
}

pub struct Visible {
  pub(crate) wanted: Vec<TileKey>,
  pub(crate) missing: Vec<TileKey>,
}

pub struct Update {
  pub complete: bool,
  pub request: Option<Request>,
}

pub struct Request {
  keys: Vec<TileKey>,
  urgent: usize,
}

impl Request {
  pub fn keys(&self) -> &[TileKey] {
    &self.keys
  }
  pub fn urgent(&self) -> usize {
    self.urgent
  }
}

impl Default for Tiles {
  fn default() -> Self {
    Self { wanted: Vec::new(), heading: 1.0 }
  }
}

impl Tiles {
  pub fn invalidate(&mut self) {
    self.wanted.clear();
  }

  pub fn wanted(&self) -> &[TileKey] {
    &self.wanted
  }

  pub fn wants(&self, key: &TileKey) -> bool {
    self.wanted.contains(key)
  }

  pub fn nearby(&self, view: View<'_>, limit: usize) -> Vec<PageId> {
    let anchor = view.layout.page_at(view.target.unwrap_or(view.scroll));
    nearby(view.layout.pages().len(), anchor, self.heading >= 0.0, limit)
  }

  pub fn scene<'a, S: Surface<'a>>(
    &mut self,
    view: View<'_>,
    frame: Frame,
    scene: &mut S,
  ) -> Result<Update, S::Error> {
    let mut wanted = visible(view.layout, view.scroll);
    if let Some(target) = view.target {
      self.heading = (target - view.scroll).signum();
      for key in visible(view.layout, target) {
        if !wanted.contains(&key) {
          wanted.push(key);
        }
      }
    }
    let mut missing = Vec::new();
    for key in &wanted {
      let (x, y) = view.layout.origin(key.page, view.scroll);
      if !scene.tile(
        TileId { owner: TileOwner::new(frame.pane), key: *key },
        (x + key.x as f32, y + key.y as f32),
        frame.screen,
        frame.left,
      )? {
        missing.push(*key);
      }
    }
    let heading = self.heading;
    self.update_with(Visible { wanted, missing }, |wanted, missing| {
      missing.sort_by_key(|key| {
        let page = &view.layout.pages()[key.page.0];
        ((page.top + key.y as f32 - view.scroll).abs() as u32, key.x)
      });
      let urgent = missing.len();
      for key in view.ahead(wanted, heading) {
        if !scene.has(TileId { owner: TileOwner::new(frame.pane), key })? {
          missing.push(key);
        }
      }
      Ok(urgent)
    })
  }

  pub fn update(&mut self, frame: Visible) -> Update {
    match self.update_with::<std::convert::Infallible>(frame, |_, missing| {
      Ok(missing.len())
    }) {
      Ok(update) => update,
      Err(never) => match never {},
    }
  }

  fn update_with<E>(
    &mut self,
    mut frame: Visible,
    collect: impl FnOnce(&[TileKey], &mut Vec<TileKey>) -> Result<usize, E>,
  ) -> Result<Update, E> {
    let complete = frame.missing.is_empty();
    let request = if frame.wanted == self.wanted {
      None
    } else {
      let urgent = collect(&frame.wanted, &mut frame.missing)?;
      self.wanted = frame.wanted;
      Some(Request { keys: frame.missing, urgent })
    };
    Ok(Update { complete, request })
  }
}

impl View<'_> {
  fn ahead(&self, near: &[TileKey], heading: f32) -> Vec<TileKey> {
    let base = self.target.unwrap_or(self.scroll);
    let height = self.layout.screen().height();
    let mut keys = Vec::new();
    for step in [heading, -heading] {
      let scroll = self.layout.clamp_scroll(base + step * height);
      let mut screen = visible(self.layout, scroll);
      if step < 0.0 {
        screen.reverse();
      }
      for key in screen {
        if !near.contains(&key) && !keys.contains(&key) {
          keys.push(key);
        }
      }
    }
    keys
  }
}

fn nearby(
  count: usize,
  anchor: PageId,
  forward: bool,
  limit: usize,
) -> Vec<PageId> {
  let mut pages = Vec::with_capacity(limit);
  for distance in 0..limit {
    let next = anchor.0.checked_add(distance).filter(|page| *page < count);
    let previous = anchor.0.checked_sub(distance);
    let sides = if forward { [next, previous] } else { [previous, next] };
    for page in sides.into_iter().flatten().map(PageId) {
      if !pages.contains(&page) {
        pages.push(page);
      }
      if pages.len() == limit {
        return pages;
      }
    }
  }
  pages
}

#[cfg(test)]
mod tests;
