use std::str::FromStr;
use vpdf_model::text::Bounds;

/// A Kitty graphics command and the cell it was placed at.
pub struct Command<'a> {
  /// One-based row and column of the cursor move before it.
  pub at: Option<(u16, u16)>,
  control: &'a str,
}

impl<'a> Command<'a> {
  pub fn pairs(&self) -> impl Iterator<Item = (&'a str, &'a str)> {
    self.control.split(',').filter_map(|pair| pair.split_once('='))
  }

  pub fn get(&self, key: &str) -> Option<&'a str> {
    self.pairs().find(|&(name, _)| name == key).map(|(_, value)| value)
  }

  pub fn number<T: FromStr>(&self, key: &str) -> Option<T> {
    self.get(key)?.parse().ok()
  }

  /// Covered pixels from `origin`, on 8×16 cells.
  pub fn bounds(&self, origin: (u16, u16)) -> Option<Bounds> {
    let (row, column) = self.at?;
    let field = |key| self.number::<f32>(key);
    let cell = |at: u16, origin: u16| f32::from(at) - 1.0 - f32::from(origin);
    let x0 = cell(column, origin.0) * 8.0 + field("X")?;
    let y0 = cell(row, origin.1) * 16.0 + field("Y")?;
    let (x1, y1) = (x0 + field("w")?, y0 + field("h")?);
    Some(Bounds { x0, y0, x1, y1 })
  }
}

/// Graphics commands in `text`, tmux passthrough included.
pub fn commands(text: &str) -> Vec<Command<'_>> {
  let mut parts = text.split("_G");
  let mut before = parts.next().unwrap_or_default();
  parts
    .map(|part| {
      // tmux doubles each ESC in passthrough.
      let moves = ["\x1b7\x1b[", "\x1b\x1b7\x1b\x1b["];
      let start = moves.iter().filter_map(|m| Some(before.rfind(m)? + m.len()));
      let at = start.max().and_then(|start| {
        let (row, column) =
          before[start..].split_once('H')?.0.split_once(';')?;
        Some((row.parse().ok()?, column.parse().ok()?))
      });
      before = part;
      Command { at, control: part.split(';').next().unwrap_or_default() }
    })
    .collect()
}
