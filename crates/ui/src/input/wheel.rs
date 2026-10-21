use crate::paint::Frame;
use crate::pointer::{Region, Route, WHEEL_ROWS, Wheel as Direction};
use vpdf_model::input::KeyModifiers;

const ZOOM: f32 = 0.5;

#[derive(Default)]
pub struct Wheel {
  run: Option<Run>,
}

#[derive(Clone, Copy)]
struct Run {
  surface: Surface,
  sideways: bool,
}

#[derive(Clone, Copy)]
enum Surface {
  Page,
  Preview,
}

#[derive(Debug, PartialEq)]
pub enum Turn {
  None,
  Zoom { steps: f32, at: (f32, f32) },
  Pan(f32),
  Scroll(f32),
  Preview(f32),
}

impl Wheel {
  pub fn cancel(&mut self) {
    self.run = None;
  }

  pub fn turn(&mut self, route: Route, frame: Frame, preview: bool) -> Turn {
    let mouse = route.mouse;
    let Some(turn) = Direction::of(mouse.kind) else { return Turn::None };
    let step = if turn.forward() { 1.0 } else { -1.0 };
    let rows = f32::from(WHEEL_ROWS);
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
        steps: -step * ZOOM,
        at: frame.point(mouse.column, mouse.row, route.pixel),
      };
    }
    match (surface, sideways) {
      (Surface::Preview, false) => Turn::Preview(step * rows),
      (Surface::Preview, true) => Turn::None,
      (Surface::Page, true) => {
        Turn::Pan(step * rows * f32::from(frame.screen.cell_width))
      }
      (Surface::Page, false) => {
        Turn::Scroll(step * rows * f32::from(frame.screen.cell_height))
      }
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
