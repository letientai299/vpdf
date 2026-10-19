//! The scrollbar over the page's right edge.

use crate::Result;
use crate::app::App;
use crate::document::Target;
use crate::graphics::Graphics;
use crate::pointer::{Hit, Route};
use crate::text::Bounds;
use crate::theme;
use crossterm::event::MouseEventKind;
use std::io::Write;
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;

/// The bar's time after a move.
pub(crate) const SHOWN: Duration = Duration::from_secs(1);

/// Tick height and spacing, in pixels.
const TICK: f32 = 3.0;

/// A tick run: top, bottom, and color.
type Run = (f32, f32, [u8; 4]);

/// What ticks depend on.
type TickKey = (usize, u64, usize, bool, u32, u32);

/// The bar's state between frames.
#[derive(Debug, Default)]
pub struct Bar {
  /// Scroll the last frame showed.
  pub(crate) seen: Option<f32>,
  /// Shown until then, after a move.
  pub(crate) until: Option<Instant>,
  pub(crate) scrub: Option<Scrub>,
  /// Tick runs and their key.
  runs: Option<(TickKey, Vec<Run>)>,
}

/// A press dragging the thumb.
#[derive(Clone, Copy, Debug)]
pub struct Scrub {
  /// Pointer row, for the label.
  row: u16,
  /// Pixels from thumb top to pointer.
  grip: f32,
  /// A grip's start, recorded once moved.
  origin: Option<vpdf_reader::reader::Location>,
}

impl App {
  /// Whether the PDF view scrolls.
  fn has_bar(&self) -> bool {
    !self.text_mode
      && self.overview.is_none()
      && !self.covered()
      && self.layout.total > self.layout.screen.height()
  }

  /// Whether a column is the track.
  pub(crate) fn on_bar(&self, column: u16) -> bool {
    // Overlays take presses there.
    self.has_bar() && column + 1 == self.screen.columns && !self.overlaid()
  }

  /// When a moved view's bar hides.
  pub fn bar_until(&self) -> Option<Instant> {
    self.bar.until
  }

  /// Hides an expired bar; whether drawn.
  pub fn bar_expired(&mut self, now: Instant) -> bool {
    let expired = self.bar.until.is_some_and(|until| now >= until);
    if expired {
      self.bar.until = None;
    }
    expired
  }

  /// Ends a scrub as a release would.
  pub(crate) fn drop_scrub(&mut self) -> bool {
    let Some(scrub) = self.bar.scrub.take() else { return false };
    // Moved thumbs record their start.
    if let Some(origin) = scrub.origin
      && origin.scroll.to_bits() != self.reader.scroll.to_bits()
    {
      self.reader.record(origin);
    }
    true
  }

  /// Scroll range the thumb travels.
  fn room(&self) -> f32 {
    (self.layout.total - self.layout.screen.height()).max(1.0)
  }

  /// The scrub's pointer row.
  pub(crate) fn scrub_row(&self) -> Option<u16> {
    self.bar.scrub.map(|scrub| scrub.row)
  }

  /// Thumb top, size, and travel in pixels.
  fn thumb(&self, height: f32) -> (f32, f32, f32) {
    let view = self.layout.screen.height();
    let size = view / self.layout.total * height;
    // Small thumbs stay grabbable.
    let size = size.max(f32::from(self.screen.cell_height)).min(height);
    let travel = height - size;
    (self.reader.scroll / self.room() * travel, size, travel)
  }

  /// Presses jump, but grip the thumb.
  pub(crate) fn bar_mouse(&mut self, route: &Route) -> Result<()> {
    let row = route.mouse.row;
    let at = self.cell_point(route.mouse.column, row, route.pixel).1;
    let height = self.uncovered().height();
    let (top, size, travel) = self.thumb(height);
    let scrub = match (route.mouse.kind, self.bar.scrub) {
      (MouseEventKind::Up(_), _) => {
        self.drop_scrub();
        return Ok(());
      }
      (MouseEventKind::Down(_), _) => {
        // Cell reports grip anywhere in the cell.
        let cell = f32::from(self.screen.cell_height);
        let grips = match route.pixel {
          Some(_) => (top..top + size).contains(&at),
          None => at + cell / 2.0 > top && at - cell / 2.0 < top + size,
        };
        if grips {
          let origin = Some(self.location()?);
          self.bar.scrub = Some(Scrub { row, grip: at - top, origin });
          return Ok(());
        }
        let origin = self.location()?;
        self.reader.record(origin);
        Scrub { row, grip: size / 2.0, origin: None }
      }
      (_, Some(scrub)) => Scrub { row, ..scrub },
      (_, None) => return Ok(()),
    };
    self.bar.scrub = Some(scrub);
    let fraction = ((at - scrub.grip) / travel.max(1.0)).clamp(0.0, 1.0);
    self.animation = None;
    self.reader.scroll = self.layout.clamp_scroll(fraction * self.room());
    Ok(())
  }

  /// Draws track, ticks, thumb, and label.
  pub(crate) fn draw_bar(
    &mut self,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<()> {
    let now = Instant::now();
    let scroll = self.reader.scroll;
    if self.bar.seen.is_some_and(|seen| seen.to_bits() != scroll.to_bits()) {
      self.bar.until = Some(now + SHOWN);
    }
    self.bar.seen = Some(scroll);
    let resting = self.hover == Hit::Scrollbar;
    let moving = self.bar.until.is_some_and(|until| now < until);
    if !self.has_bar() || !(resting || moving || self.bar.scrub.is_some()) {
      return Ok(());
    }
    let screen = self.uncovered();
    let height = screen.height();
    let column = screen.columns.saturating_sub(1);
    let rows = screen.rows.saturating_sub(1);
    let left = self.left;
    let track = (column, 1, rows);
    graphics.stretch(theme::TRACK, -1, track, screen, left, out)?;
    let x1 = screen.width();
    let x0 = x1 - f32::from(screen.cell_width);
    let rect = |y0: f32, y1: f32| Bounds { x0, y0, x1, y1 };
    self.refresh_runs(height);
    let cached = self.bar.runs.take();
    for &(y0, y1, color) in cached.iter().flat_map(|(_, runs)| runs) {
      graphics.backdrop(rect(y0, y1), color, screen, left, out)?;
    }
    self.bar.runs = cached;
    let (top, size, _) = self.thumb(height);
    let thumb = rect(top, top + size);
    graphics.backdrop(thumb, theme::THUMB, screen, left, out)?;
    if let Some(scrub) = self.bar.scrub {
      self.draw_scrub_label(graphics, scrub.row, out)?;
    }
    Ok(())
  }

  /// Rebuilds stale tick runs.
  fn refresh_runs(&mut self, height: f32) {
    let key = (
      self.doc.outline.len(),
      self.search.generation(),
      self.search.hits().len(),
      self.search.hidden(),
      self.layout.total.to_bits(),
      height.to_bits(),
    );
    if self.bar.runs.as_ref().is_none_or(|(cached, _)| *cached != key) {
      self.bar.runs = Some((key, runs(self.ticks(height))));
    }
  }

  /// Ticks for headings and matched pages.
  fn ticks(&self, height: f32) -> Vec<(f32, [u8; 4])> {
    let total = self.layout.total;
    let at = |y: f32| ((y / total * height) / TICK).floor() * TICK;
    let mut ticks = Vec::new();
    let headings = self.doc.outline.iter().filter(|item| item.depth == 0);
    for heading in headings {
      let y = match heading.target {
        Target::Page(page) => self.layout.pages[page.0].top,
        Target::Point { page, y, .. } => {
          let point = Bounds { x0: 0.0, x1: 0.0, y0: y, y1: y };
          self.layout.project(page, point, 0.0).y0
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
      let layout = &self.layout.pages[page.0];
      ticks.push((at(layout.top + layout.height / 2.0), theme::MATCH_TICK));
      index = hits.partition_point(|hit| hit.first.page <= page);
    }
    ticks.sort_by(|a, b| a.0.total_cmp(&b.0));
    ticks.dedup_by(|a, b| a.0.to_bits() == b.0.to_bits());
    ticks
  }

  /// The scrubbed page, left of the track.
  fn draw_scrub_label(
    &mut self,
    graphics: &mut Graphics,
    row: u16,
    out: &mut impl Write,
  ) -> Result<()> {
    let screen = self.uncovered();
    let y = f32::from(row) * f32::from(screen.cell_height);
    let page = self.layout.page_at(self.reader.scroll + y);
    let label = format!(" {} ", self.page_line(page));
    let width = u16::try_from(label.width()).unwrap_or(u16::MAX);
    let columns = screen.columns.saturating_sub(1);
    let start = columns.saturating_sub(width);
    let label = crate::terminal::clean(&label, usize::from(columns - start));
    let column = self.left + start;
    write!(
      out,
      "{}{}{label}{}",
      self.at.goto(row, column),
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
    graphics.backdrop(area, theme::BAR_FILL, screen, self.left, out)
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
mod tests {
  use super::*;
  use crate::layout::Screen;
  use crate::search::Hit as Found;
  use crate::text::{PageId, Position};

  #[test]
  fn touching_ticks_of_one_color_merge() {
    let (a, b) = ([1; 4], [2; 4]);
    // (ticks) -> runs
    let cases = [
      (vec![(0.0, a), (3.0, a)], vec![(0.0, 5.0, a)]),
      (vec![(0.0, a), (6.0, a)], vec![(0.0, 2.0, a), (6.0, 8.0, a)]),
      (vec![(0.0, a), (3.0, b)], vec![(0.0, 2.0, a), (3.0, 5.0, b)]),
      (vec![], vec![]),
    ];
    for (ticks, want) in cases {
      assert_eq!(runs(ticks.clone()), want, "{ticks:?}");
    }
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn ticks_mark_headings_and_pages_with_matches() -> Result<()> {
    let root = std::path::PathBuf::from(std::env::var("VPDF_CORPUS")?);
    let screen = Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut app = App::with_screen(&root.join("paper.pdf"), "", false, screen)?;
    app.doc.load_outline()?;
    let height = app.layout.screen.height();
    let headings = app.ticks(height);
    let top = app.doc.outline.iter().filter(|item| item.depth == 0).count();
    assert!(!headings.is_empty() && headings.len() <= top);
    assert!(headings.iter().all(|(_, color)| *color == theme::HEADING_TICK));
    let found = |page| {
      let at = Position { page: PageId(page), line: 0, glyph: 0 };
      Found { first: at, last: at, excerpt: String::new() }
    };
    // Three matches on two pages: two ticks.
    let query = crate::find::Query::default();
    app.search.show(query, vec![found(2), found(2), found(9)]);
    let matches = app.ticks(height).into_iter();
    let matches = matches.filter(|(_, color)| *color == theme::MATCH_TICK);
    let ys: Vec<_> = matches.map(|(y, _)| y).collect();
    assert_eq!(ys.len(), 2);
    assert!(ys.iter().all(|y| (0.0..height).contains(y)));
    app.search.set_hidden(true);
    assert_eq!(app.ticks(height), headings, "hidden matches mark nothing");
    Ok(())
  }
}
