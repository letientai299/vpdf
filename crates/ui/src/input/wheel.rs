use crate::paint::Frame;
use crate::pointer::{Region, Route, WHEEL_ROWS, Wheel as Direction};
use vpdf_model::input::KeyModifiers;

const ZOOM: f32 = 0.5;

#[derive(Default)]
pub struct Wheel {
  run: Option<Run>,
  precise: Option<Precise>,
}

/// A precise device's turn: a fraction of a wheel turn, and the whole
/// turns completed so far for surfaces that move in rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Precise {
  pub turns: f32,
  pub whole: u16,
}

#[derive(Clone, Copy)]
struct Run {
  surface: Surface,
  sideways: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Surface {
  Page,
  Preview,
}

#[derive(Debug, PartialEq)]
pub enum Turn {
  None,
  Zoom {
    steps: f32,
    at: (f32, f32),
  },
  Pan(f32),
  Scroll(f32),
  /// A precise scroll, applied without easing.
  Exact(f32),
  Preview(f32),
}

impl Wheel {
  pub fn cancel(&mut self) {
    self.run = None;
  }

  /// Sizes the next turn.
  pub fn precise(&mut self, precise: Precise) {
    self.precise = Some(precise);
  }

  pub fn turn(&mut self, route: Route, frame: Frame, preview: bool) -> Turn {
    let precise = self.precise.take();
    let mouse = route.mouse;
    let Some(turn) = Direction::of(mouse.kind) else { return Turn::None };
    let step = if turn.forward() { 1.0 } else { -1.0 };
    let rows = f32::from(WHEEL_ROWS);
    let (turns, whole) =
      precise.map_or((1.0, 1.0), |p| (p.turns, f32::from(p.whole)));
    let sideways =
      turn.sideways() || mouse.modifiers.contains(KeyModifiers::SHIFT);
    let zoom = mouse.modifiers.contains(KeyModifiers::CONTROL);
    let Some(surface) = self.surface(route.region, sideways, zoom, preview)
    else {
      return Turn::None;
    };
    if zoom {
      if sideways || frame.text {
        return Turn::None;
      }
      return Turn::Zoom {
        steps: -step * ZOOM * turns,
        at: frame.point(mouse.column, mouse.row, route.pixel),
      };
    }
    // Rows and text cells move in whole turns.
    let rowed = surface == Surface::Preview || frame.text;
    let size = step * rows * if rowed { whole } else { turns };
    if size == 0.0 {
      return Turn::None;
    }
    let cell =
      (f32::from(frame.screen.cell_width), f32::from(frame.screen.cell_height));
    match (surface, sideways) {
      (Surface::Preview, false) => Turn::Preview(size),
      (Surface::Preview, true) => Turn::None,
      (Surface::Page, true) => Turn::Pan(size * cell.0),
      (Surface::Page, false) if precise.is_some() && !rowed => {
        Turn::Exact(size * cell.1)
      }
      (Surface::Page, false) => Turn::Scroll(size * cell.1),
    }
  }

  fn surface(
    &mut self,
    region: Region,
    sideways: bool,
    zoom: bool,
    preview: bool,
  ) -> Option<Surface> {
    if let Some(run) = self.run {
      let surface = match run.surface {
        Surface::Preview if !preview => Surface::Page,
        surface => surface,
      };
      return (run.sideways == sideways).then_some(surface);
    }
    let under = !zoom && region == Region::Preview;
    let surface = if under { Surface::Preview } else { Surface::Page };
    self.run = Some(Run { surface, sideways });
    Some(surface)
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Background {
  Panel,
  Overview,
  Page,
}

pub fn background(
  column: u16,
  left: u16,
  covered: bool,
  overview: bool,
) -> Background {
  if covered || column < left {
    Background::Panel
  } else if overview {
    Background::Overview
  } else {
    Background::Page
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn background_follows_panel_coverage() {
    for (column, covered, overview, expected) in [
      (19, false, false, Background::Panel),
      (19, false, true, Background::Panel),
      (20, false, false, Background::Page),
      (20, false, true, Background::Overview),
      (79, true, false, Background::Panel),
      (79, true, true, Background::Panel),
    ] {
      assert_eq!(background(column, 20, covered, overview), expected);
    }
  }
}
