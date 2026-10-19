use crate::Result;
use crate::find::Search;
use crate::layout::Layout;
use crate::paint::Frame;
use crate::pointer::Route;
use crate::text::{Bounds, PageId};
use crate::theme;
use crossterm::event::MouseEventKind;
use std::io::Write;
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;
use vpdf_model::document::{Heading, Target};
use vpdf_reader::reader::{Location, Reader};
use vpdf_terminal::graphics::Graphics;

const TICK: f32 = 3.0;
type Run = (f32, f32, [u8; 4]);
type TickKey = (usize, u64, usize, bool, u32, u32);

#[derive(Debug, Default)]
pub struct Bar {
  seen: Option<f32>,
  until: Option<Instant>,
  scrub: Option<Scrub>,
  runs: Option<(TickKey, Vec<Run>)>,
}

#[derive(Clone, Copy, Debug)]
struct Scrub {
  row: u16,
  grip: f32,
  origin: Option<Location>,
}

#[derive(Clone, Copy)]
pub struct View<'a> {
  pub layout: &'a Layout,
  pub scroll: f32,
  pub frame: Frame,
  pub available: bool,
  pub hovered: bool,
}

#[derive(Clone, Copy)]
pub struct Ticks<'a> {
  pub outline: &'a [Heading],
  pub search: &'a Search,
}

pub enum Input {
  None,
  Release,
  Press(Press),
  Scroll(f32),
}

/// A press awaiting its reading location.
pub struct Press {
  row: u16,
  grip: f32,
  scroll: Option<f32>,
}

/// A label awaiting native section text.
pub struct Label {
  page: PageId,
  row: u16,
  frame: Frame,
}

impl View<'_> {
  fn room(&self) -> f32 {
    (self.layout.total() - self.layout.screen().height()).max(1.0)
  }

  fn thumb(&self) -> (f32, f32, f32) {
    let height = self.frame.screen.height();
    let view = self.layout.screen().height();
    let size = view / self.layout.total() * height;
    let size = size.max(f32::from(self.frame.screen.cell_height)).min(height);
    let travel = height - size;
    (self.scroll / self.room() * travel, size, travel)
  }

  fn scrolled(&self, at: f32, grip: f32, travel: f32) -> f32 {
    let fraction = ((at - grip) / travel.max(1.0)).clamp(0.0, 1.0);
    self.layout.clamp_scroll(fraction * self.room())
  }
}

impl Bar {
  pub const SHOWN: Duration = Duration::from_secs(1);

  pub fn deadline(&self) -> Option<Instant> {
    self.until
  }

  pub fn scrubbing(&self) -> bool {
    self.scrub.is_some()
  }

  pub fn row(&self) -> Option<u16> {
    self.scrub.map(|scrub| scrub.row)
  }

  pub fn observe(&mut self, scroll: f32, now: Instant) {
    if self.seen.is_some_and(|seen| seen.to_bits() != scroll.to_bits()) {
      self.until = Some(now + Self::SHOWN);
    }
    self.seen = Some(scroll);
  }

  pub fn reflow(&mut self, before: f32, after: f32) {
    if self.seen.is_some_and(|seen| seen.to_bits() == before.to_bits()) {
      self.seen = Some(after);
    }
  }

  pub fn expire(&mut self, now: Instant) -> bool {
    let expired = self.until.is_some_and(|until| now >= until);
    if expired {
      self.until = None;
    }
    expired
  }

  pub fn release(&mut self, reader: &mut Reader) -> bool {
    let Some(scrub) = self.scrub.take() else { return false };
    if let Some(origin) = scrub.origin
      && origin.scroll.to_bits() != reader.scroll.to_bits()
    {
      reader.record(origin);
    }
    true
  }

  pub fn input(&mut self, view: &View<'_>, route: &Route) -> Input {
    let row = route.mouse.row;
    let at = view.frame.point(route.mouse.column, row, route.pixel).1;
    let (top, size, travel) = view.thumb();
    let scrub = match (route.mouse.kind, self.scrub) {
      (MouseEventKind::Up(_), _) => return Input::Release,
      (MouseEventKind::Down(_), _) => {
        let cell = f32::from(view.frame.screen.cell_height);
        let grips = match route.pixel {
          Some(_) => (top..top + size).contains(&at),
          None => at + cell / 2.0 > top && at - cell / 2.0 < top + size,
        };
        let grip = if grips { at - top } else { size / 2.0 };
        return Input::Press(Press {
          row,
          grip,
          scroll: (!grips).then(|| view.scrolled(at, grip, travel)),
        });
      }
      (_, Some(scrub)) => Scrub { row, ..scrub },
      (_, None) => return Input::None,
    };
    self.scrub = Some(scrub);
    Input::Scroll(view.scrolled(at, scrub.grip, travel))
  }

  pub fn draw(
    &mut self,
    view: View<'_>,
    ticks: Ticks<'_>,
    now: Instant,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<Option<Label>> {
    self.observe(view.scroll, now);
    let moving = self.until.is_some_and(|until| now < until);
    if !view.available || !(view.hovered || moving || self.scrubbing()) {
      return Ok(None);
    }
    let screen = view.frame.screen;
    let height = screen.height();
    let column = screen.columns.saturating_sub(1);
    let rows = screen.rows.saturating_sub(1);
    let left = view.frame.left;
    let track = (column, 1, rows);
    graphics.stretch(theme::TRACK, -1, track, screen, left, out)?;
    let x1 = screen.width();
    let x0 = x1 - f32::from(screen.cell_width);
    let rect = |y0: f32, y1: f32| Bounds { x0, y0, x1, y1 };
    let key = (
      ticks.outline.len(),
      ticks.search.generation(),
      ticks.search.hits().len(),
      ticks.search.hidden(),
      view.layout.total().to_bits(),
      height.to_bits(),
    );
    if self.runs.as_ref().is_none_or(|(cached, _)| *cached != key) {
      self.runs = Some((key, runs(ticks.positions(view.layout, height))));
    }
    let cached = self.runs.take();
    for &(y0, y1, color) in cached.iter().flat_map(|(_, runs)| runs) {
      graphics.backdrop(rect(y0, y1), color, screen, left, out)?;
    }
    self.runs = cached;
    let (top, size, _) = view.thumb();
    graphics.backdrop(
      rect(top, top + size),
      theme::THUMB,
      screen,
      left,
      out,
    )?;
    Ok(self.scrub.map(|scrub| {
      let y = f32::from(scrub.row) * f32::from(screen.cell_height);
      Label {
        page: view.layout.page_at(view.scroll + y),
        row: scrub.row,
        frame: view.frame,
      }
    }))
  }
}

impl Press {
  pub fn apply(
    self,
    bar: &mut Bar,
    origin: Location,
    reader: &mut Reader,
  ) -> Option<f32> {
    let origin = if self.scroll.is_some() {
      reader.record(origin);
      None
    } else {
      Some(origin)
    };
    bar.scrub = Some(Scrub { row: self.row, grip: self.grip, origin });
    self.scroll
  }
}

impl Ticks<'_> {
  /// Ticks for headings and matched pages.
  pub fn positions(&self, layout: &Layout, height: f32) -> Vec<(f32, [u8; 4])> {
    let total = layout.total();
    let at = |y: f32| ((y / total * height) / TICK).floor() * TICK;
    let mut ticks = Vec::new();
    let headings = self.outline.iter().filter(|item| item.depth == 0);
    for heading in headings {
      let y = match heading.target {
        Target::Page(page) => layout.pages()[page.0].top,
        Target::Point { page, y, .. } => {
          let point = Bounds { x0: 0.0, x1: 0.0, y0: y, y1: y };
          layout.project(page, point, 0.0).y0
        }
        Target::Uri(_) => continue,
      };
      ticks.push((at(y), theme::HEADING_TICK));
    }
    let hits = self.search.hits();
    let mut index = 0;
    // Hits carry no y: mark their pages.
    while !self.search.hidden() && index < hits.len() {
      let page = hits[index].first.page;
      let layout = &layout.pages()[page.0];
      ticks.push((at(layout.top + layout.height / 2.0), theme::MATCH_TICK));
      index = hits.partition_point(|hit| hit.first.page <= page);
    }
    ticks.sort_by(|a, b| a.0.total_cmp(&b.0));
    ticks.dedup_by(|a, b| a.0.to_bits() == b.0.to_bits());
    ticks
  }
}

impl Label {
  pub fn page(&self) -> PageId {
    self.page
  }

  pub fn draw(
    self,
    text: &str,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<()> {
    let screen = self.frame.screen;
    let row = self.row;
    let y = f32::from(row) * f32::from(screen.cell_height);
    let label = format!(" {text} ");
    let width = u16::try_from(label.width()).unwrap_or(u16::MAX);
    let columns = screen.columns.saturating_sub(1);
    let start = columns.saturating_sub(width);
    let label =
      vpdf_terminal::terminal::clean(&label, usize::from(columns - start));
    let column = self.frame.left + start;
    write!(
      out,
      "{}{}{label}{}",
      self.frame.at.goto(row, column),
      theme::BAR,
      theme::RESET
    )?;
    let (cell_width, cell_height) =
      (f32::from(screen.cell_width), f32::from(screen.cell_height));
    let x0 = f32::from(start) * cell_width;
    let area = Bounds {
      x0,
      x1: x0 + f32::from(columns - start) * cell_width,
      y0: y,
      y1: y + cell_height,
    };
    graphics.backdrop(area, theme::BAR_FILL, screen, self.frame.left, out)
  }
}

/// Merges touching same-color ticks.
fn runs(ticks: Vec<(f32, [u8; 4])>) -> Vec<Run> {
  let mut runs: Vec<Run> = Vec::new();
  for (y, color) in ticks {
    match runs.last_mut() {
      Some(run) if run.2 == color && y <= run.1 + 1.0 => run.1 = y + TICK - 1.0,
      _ => runs.push((y, y + TICK - 1.0, color)),
    }
  }
  runs
}

#[cfg(test)]
mod tests;
