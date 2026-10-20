use crate::Result;
use crate::layout::Screen;
use std::io::Write;
use vpdf_model::scene::{Draw, Op};
use vpdf_model::style::Role;
use vpdf_terminal::terminal::At;

const CELLS: usize = 4;
const WIDTH: u16 = 16;

#[derive(Clone, Copy)]
pub struct Snapshot {
  pub fps: f64,
  pub cpu: Option<f64>,
  pub rss: Option<u64>,
  pub bytes_per_second: f64,
}

pub struct Overlay {
  cells: Vec<String>,
  painted: Vec<String>,
  layout: Option<(u16, u16)>,
  changed: bool,
}

impl Default for Overlay {
  fn default() -> Self {
    Self { cells: Vec::new(), painted: Vec::new(), layout: None, changed: true }
  }
}

impl Overlay {
  pub fn update(&mut self, sample: Snapshot) {
    self.changed = true;
    self.cells = vec![
      format!("FPS {:.0}", sample.fps),
      number("CPU", sample.cpu, "%"),
      number("RSS", sample.rss.map(|rss| rss as f64 / 1_048_576.0), "M"),
      format!("Out {:.1}K/s", sample.bytes_per_second / 1024.0),
    ];
  }

  pub fn invalidate(&mut self) {
    self.layout = None;
  }

  pub fn draw(
    &mut self,
    out: &mut impl Write,
    screen: Screen,
    force: bool,
  ) -> Result<bool> {
    let mut scene = vpdf_terminal::painter::Painter::new(out);
    let changed = self.scene(&mut scene, screen, force);
    scene.finish()?;
    Ok(changed)
  }

  pub fn scene<'a>(
    &mut self,
    scene: &mut impl Draw<'a>,
    screen: Screen,
    force: bool,
  ) -> bool {
    let layout = (screen.columns, screen.rows);
    if !force && !self.changed && self.layout == Some(layout) {
      return false;
    }
    let force = force || self.layout != Some(layout);
    self.layout = Some(layout);
    self.changed = false;
    let lines = self.lines(screen);
    if !force && lines == self.painted {
      return false;
    }
    scene.extend([Op::SaveCursor, Op::Style(Role::Bar)]);
    for (row, line) in lines.iter().enumerate() {
      crate::paint::line(
        scene,
        At::default(),
        row as u16,
        line,
        screen.columns,
      );
    }
    scene.extend([Op::Style(Role::Reset), Op::RestoreCursor]);
    self.painted = lines;
    true
  }

  fn lines(&self, screen: Screen) -> Vec<String> {
    let columns = usize::from((screen.columns / WIDTH).max(1));
    let width = usize::from(screen.columns.min(WIDTH));
    let mut cells = self.cells.clone();
    if cells.is_empty() {
      cells =
        ["FPS", "CPU", "RSS", "Out"].map(|label| format!("{label} —")).to_vec();
    }
    cells
      .chunks(columns)
      .take(usize::from(rows(screen)))
      .map(|chunk| {
        chunk.iter().fold(String::new(), |mut line, cell| {
          let cell: String = cell.chars().take(width).collect();
          let padding = width.saturating_sub(cell.chars().count());
          line.push_str(&cell);
          line.extend(std::iter::repeat_n(' ', padding));
          line
        })
      })
      .collect()
  }
}

pub fn rows(screen: Screen) -> u16 {
  let columns = (screen.columns / WIDTH).max(1);
  (CELLS as u16).div_ceil(columns).min(screen.rows.saturating_sub(3))
}

fn number(label: &str, value: Option<f64>, unit: &str) -> String {
  value.map_or_else(
    || format!("{label} —"),
    |value| format!("{label} {value:.1}{unit}"),
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  fn screen(columns: u16, rows: u16) -> Screen {
    Screen { columns, rows, cell_width: 8, cell_height: 16, origin: (0, 0) }
  }

  #[test]
  fn wraps_and_preserves_content() {
    let monitor = Overlay::default();
    for width in [22, 40, 80, 120, 286, 400] {
      let screen = screen(width, 30);
      let lines = monitor.lines(screen);
      assert_eq!(lines.len(), usize::from(rows(screen)));
      assert!(lines.iter().all(|line| line.chars().count() <= width as usize));
      let text = lines.join(" ");
      for label in ["FPS", "CPU", "RSS", "Out"] {
        assert_eq!(text.matches(label).count(), 1, "{width}: {label}");
      }
    }
    for height in 0..8 {
      for width in 0..22 {
        let screen = screen(width, height);
        assert!(rows(screen) <= height.saturating_sub(3));
        assert!(
          monitor
            .lines(screen)
            .iter()
            .all(|line| line.chars().count() <= width as usize)
        );
      }
    }
  }

  #[test]
  fn samples_invalidate_only_changed_output() -> Result<()> {
    let mut overlay = Overlay::default();
    let sample = || Snapshot {
      fps: 120.0,
      cpu: Some(234.5),
      rss: Some(1_099_431_936),
      bytes_per_second: 1_048_780.8,
    };
    let screen = screen(80, 30);
    overlay.update(sample());
    let mut out = Vec::new();
    assert!(overlay.draw(&mut out, screen, false)?);
    let text = String::from_utf8_lossy(&out);
    for value in ["FPS 120", "CPU 234.5%", "RSS 1048.5M", "Out 1024.2K/s"] {
      assert!(text.contains(value), "{value}");
    }
    let first = out.clone();
    out.clear();
    overlay.update(sample());
    assert!(!overlay.draw(&mut out, screen, false)?);
    assert_eq!(out, [] as [u8; 0]);
    overlay.invalidate();
    assert!(overlay.draw(&mut out, screen, false)?);
    assert_eq!(out, first);
    out.clear();
    assert!(overlay.draw(&mut out, screen, true)?);
    assert_eq!(out, first);
    Ok(())
  }

  #[test]
  fn repaint_tracks_content_and_size() -> Result<()> {
    let mut overlay = Overlay::default();
    let mut out = Vec::new();
    assert!(overlay.draw(&mut out, screen(80, 30), false)?);
    assert!(String::from_utf8_lossy(&out).contains("\x1b[1;1H"));
    out.clear();
    assert!(!overlay.draw(&mut out, screen(80, 30), false)?);
    assert_eq!(out, [] as [u8; 0]);
    assert!(overlay.draw(&mut out, screen(80, 31), false)?);
    assert!(String::from_utf8(out)?.contains("\x1b8"));
    Ok(())
  }
}
