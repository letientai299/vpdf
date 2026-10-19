use crate::paint::Frame;
use crate::pointer::Route;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Edge {
  held: Option<Held>,
}

#[derive(Clone, Copy)]
struct Held {
  route: Route,
  rows: isize,
  due: Instant,
}

pub struct Start {
  route: Route,
  rows: isize,
}

pub struct Step {
  pub route: Route,
  pub rows: isize,
}

impl Edge {
  pub const INTERVAL: Duration = Duration::from_millis(50);

  pub fn deadline(&self) -> Option<Instant> {
    self.held.map(|held| held.due)
  }

  pub fn cancel(&mut self) {
    self.held = None;
  }

  pub fn follow(&mut self, route: Route, rows: isize) -> Option<Start> {
    if rows == 0 {
      self.cancel();
      return None;
    }
    if let Some(held) = &mut self.held {
      held.route = route;
      held.rows = rows;
      return None;
    }
    Some(Start { route, rows })
  }

  pub fn step(&mut self, now: Instant) -> Option<Step> {
    let held = self.held.filter(|held| now >= held.due)?;
    let late =
      now.duration_since(held.due).as_millis() / Self::INTERVAL.as_millis();
    let steps = 1 + late.min(3) as isize;
    self.held = Some(Held { due: now + Self::INTERVAL, ..held });
    Some(Step { route: held.route, rows: held.rows * steps })
  }
}

impl Start {
  pub fn rows(&self) -> isize {
    self.rows
  }

  pub fn begin(self, edge: &mut Edge, now: Instant) {
    edge.held = Some(Held {
      route: self.route,
      rows: self.rows,
      due: now + Edge::INTERVAL,
    });
  }
}

pub fn rows(frame: Frame, covered: u16, route: &Route) -> isize {
  let last = frame.screen.rows.saturating_sub(2 + covered);
  let row = route.mouse.row;
  let Some(pixel) = route.pixel else {
    return match row {
      _ if last == 0 => 0,
      0 => -1,
      _ if row >= last => 1,
      _ => 0,
    };
  };
  let height = i32::from(pixel.cell.1.max(1));
  let top = i32::from(frame.at.row) * height;
  let bottom = (i32::from(frame.at.row) + i32::from(last) + 1) * height;
  let y = i32::from(pixel.y);
  let (direction, past) = if y < top || (y == 0 && top == 0 && row == 0) {
    (-1, (top - y).max(1))
  } else if y >= bottom {
    (1, y - bottom + 1)
  } else {
    return 0;
  };
  direction * (1 + (past - 1) / height) as isize
}
