use crate::Result;
use crate::app::App;
use crate::document::Target;
use crate::keys;
use crate::pointer::Items;
use crate::split::Rect;
use crate::text::{Bounds, PageId};
use crate::theme;
use crossterm::event::{KeyCode, KeyEvent};
use std::io::Write;
use vpdf_terminal::graphics::Graphics;

/// Home-row first, like Vimium's default hint characters.
const ALPHABET: &[u8] = b"asdfjklghqweruiopzxcvnm";

pub struct Hint {
  pub label: String,
  pub target: Target,
  /// The link's first shown cell, zero-based.
  pub cell: (u16, u16),
  /// The label's first column.
  pub column: u16,
}

/// Link labels on screen; typing one follows its link.
pub struct Hints {
  pub items: Vec<Hint>,
  pub typed: String,
  /// `F`: copy URIs instead of opening them.
  pub copy: bool,
  /// The pointer's label; Enter runs it.
  pub selected: Option<usize>,
}

/// `count` labels of equal length, so no label prefixes another.
pub fn labels(count: usize) -> Vec<String> {
  let base = ALPHABET.len();
  let mut length = 1;
  while base.pow(length) < count {
    length += 1;
  }
  (0..count)
    .map(|mut index| {
      let mut label = vec![0; length as usize];
      for slot in label.iter_mut().rev() {
        *slot = ALPHABET[index % base];
        index /= base;
      }
      String::from_utf8_lossy(&label).into_owned()
    })
    .collect()
}

fn width(label: &str) -> u16 {
  u16::try_from(label.len()).unwrap_or(u16::MAX)
}

/// Shown links: first cell, last column.
type Spot = ((u16, u16), u16);

/// Cells a label may take, by row.
struct Room<'a> {
  spots: &'a [Spot],
  /// Placed labels: row, first and last column.
  placed: Vec<(u16, u16, u16)>,
  left: u16,
  right: u16,
}

impl Room<'_> {
  /// Left of link `index`, else nearby.
  fn place(&mut self, index: usize, width: u16) -> u16 {
    let ((column, row), _) = self.spots[index];
    let fit =
      |from: u16| from.min(self.right.saturating_sub(width)).max(self.left);
    let free = |from: u16| {
      let to = from + width.saturating_sub(1);
      let links = self.spots.iter().enumerate().any(|(other, spot)| {
        let ((start, on), end) = *spot;
        other != index && on == row && start <= to && end >= from
      });
      let labels = (self.placed.iter())
        .any(|&(on, start, end)| on == row && start <= to && end >= from);
      !links && !labels
    };
    let left = column.saturating_sub(width);
    // Nearest free span: left, over, then outward.
    let right = (column + 1..self.right).chain((0..left).rev());
    let from = [left, column]
      .into_iter()
      .chain(right)
      .map(fit)
      .find(|&from| free(from))
      .unwrap_or_else(|| fit(column));
    self.placed.push((row, from, from + width.saturating_sub(1)));
    from
  }
}

impl App {
  /// Labels links in view; `copy` copies URIs instead of opening them.
  pub fn start_hints(&mut self, copy: bool) -> Result<()> {
    let mut found = Vec::new();
    let pages = if self.text_mode {
      self.text_pages()?
    } else {
      self.layout.visible(self.reader.scroll)
    };
    for page in pages {
      for link in self.doc.links(PageId(page))?.iter() {
        if let Some(spot) = self.link_spot(PageId(page), link)? {
          found.push((spot, link.target.clone()));
        }
      }
    }
    anyhow::ensure!(!found.is_empty(), "No links in view");
    found.sort_by_key(|(((column, row), _), _)| (*row, *column));
    let spots: Vec<_> = found.iter().map(|(spot, _)| *spot).collect();
    let (left, right) = (self.left, self.screen.columns);
    let mut room = Room { spots: &spots, placed: Vec::new(), left, right };
    let items = (labels(found.len()).into_iter().zip(found).enumerate())
      .map(|(index, (label, (spot, target)))| {
        let column = room.place(index, width(&label));
        Hint { label, target, cell: spot.0, column }
      })
      .collect();
    let typed = String::new();
    self.hints = Some(Hints { items, typed, copy, selected: None });
    Ok(())
  }

  /// The first shown part's cells.
  fn link_spot(
    &mut self,
    page: PageId,
    link: &crate::document::Link,
  ) -> Result<Option<Spot>> {
    for bounds in link.boxes() {
      let Some(cell) = self.link_cell(page, bounds)? else { continue };
      let edge = Bounds { x0: bounds.x1, ..bounds };
      let end = self.link_cell(page, edge)?.map_or(cell.0, |end| end.0);
      return Ok(Some((cell, end.max(cell.0))));
    }
    Ok(None)
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
      return self.text_cell(position);
    }
    let rect = self.layout.project(page, bounds, self.reader.scroll);
    let screen = self.layout.screen;
    if rect.x1 < 0.0
      || rect.y1 < 0.0
      || rect.x0 >= screen.width()
      || rect.y0 >= screen.height()
    {
      return Ok(None);
    }
    let column = rect.x0.max(0.0) as u32 / u32::from(screen.cell_width);
    // The shown part's middle.
    let (top, bottom) = (rect.y0.max(0.0), rect.y1.min(screen.height()));
    let middle =
      f32::midpoint(top, bottom).min((screen.height() - 1.0).max(0.0));
    let row = middle.max(0.0) as u32 / u32::from(screen.cell_height);
    Ok(Some((self.left + u16::try_from(column)?, u16::try_from(row)?)))
  }

  /// Narrows by typed characters and follows a unique match.
  pub fn hint_key(&mut self, key: KeyEvent) -> Result<()> {
    let Some(hints) = &mut self.hints else { return Ok(()) };
    let key = keys::normalize(key);
    if key.code == KeyCode::Enter
      && let Some(index) = hints.selected
    {
      return self.run_hint(index);
    }
    if key.code == KeyCode::Esc {
      self.hints = None;
      self.reader.cancel();
      self.disarm();
      return Ok(());
    }
    // Typing may hide the picked label.
    hints.selected = None;
    match (keys::plain(key), key.code) {
      (true, KeyCode::Char(ch)) if ch.is_ascii_alphabetic() => {
        hints.typed.push(ch.to_ascii_lowercase());
      }
      (true, KeyCode::Backspace) if !hints.typed.is_empty() => {
        hints.typed.pop();
        return Ok(());
      }
      _ => {
        self.hints = None;
        return Ok(());
      }
    }
    let shown: Vec<_> = hints.shown().map(|(index, _)| index).take(2).collect();
    match shown[..] {
      [] => {
        let typed = std::mem::take(&mut hints.typed);
        self.hints = None;
        anyhow::bail!("No link labeled {typed}");
      }
      [index] if hints.items[index].label == hints.typed => {
        self.run_hint(index)
      }
      _ => Ok(()),
    }
  }

  /// Follows or copies a label's link.
  pub(crate) fn run_hint(&mut self, index: usize) -> Result<()> {
    let Some(hints) = self.hints.take() else { return Ok(()) };
    let Some(hint) = hints.items.into_iter().nth(index) else { return Ok(()) };
    match hint.target {
      Target::Uri(uri) if hints.copy => self.copy_address(uri),
      target => self.follow(&target),
    }
  }

  /// Pane cells of the shown labels.
  pub(crate) fn hint_items(&self) -> Items {
    let mut items = Items::default();
    let Some(hints) = &self.hints else { return items };
    for (index, hint) in hints.shown() {
      items.push(hint.cells(), index);
    }
    items
  }

  /// Draws labels; Kitty gets a backdrop image under each label's text.
  pub fn draw_hints(
    &mut self,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<()> {
    let Some(hints) = &self.hints else { return Ok(()) };
    let mut backdrops = Vec::new();
    for (index, hint) in hints.shown() {
      let picked = hints.selected == Some(index);
      let (typed, rest, fill) = if picked {
        (theme::HINT_PICKED_TYPED, theme::HINT_PICKED, theme::BADGE_FILL)
      } else {
        (theme::HINT_TYPED, theme::HINT, theme::HINT_FILL)
      };
      let cells = hint.cells();
      write!(
        out,
        "{}{typed}{}{rest}{}\x1b[0m",
        self.at.goto(cells.row, cells.column),
        hints.typed,
        &hint.label[hints.typed.len()..],
      )?;
      backdrops.push((self.cell_bounds(cells), fill));
    }
    if self.text_mode {
      return Ok(());
    }
    let screen = self.layout.screen;
    for (rect, fill) in backdrops {
      graphics.backdrop(rect, fill, screen, self.left, out)?;
    }
    Ok(())
  }
}

impl Hints {
  /// Labels the typed letters still match.
  fn shown(&self) -> impl Iterator<Item = (usize, &Hint)> {
    let typed = &self.typed;
    let all = self.items.iter().enumerate();
    all.filter(move |(_, hint)| hint.label.starts_with(typed.as_str()))
  }
}

impl Hint {
  /// The label's pane cells.
  fn cells(&self) -> Rect {
    let columns = width(&self.label);
    Rect { column: self.column, row: self.cell.1, columns, rows: 1 }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn labels_take_free_cells() {
    // `[35, 2, 5]`, then `[1,2,5]`.
    let spots = [
      ((10, 3), 11),
      ((14, 3), 14),
      ((17, 3), 17),
      ((10, 4), 10),
      ((12, 4), 12),
      ((14, 4), 14),
      ((10, 5), 10),
      ((11, 5), 11),
      ((12, 5), 12),
    ];
    // (link, width) in order -> label columns
    let cases = [
      (&[(0, 1), (1, 1), (2, 1)][..], &[9, 13, 16][..]),
      (&[(0, 2), (1, 2), (2, 2)][..], &[8, 12, 15][..]),
      // Crowded labels sit over their link.
      (&[(3, 2), (4, 2), (5, 2)][..], &[8, 12, 14][..]),
      // Adjacent links: labels move right.
      (&[(6, 2), (7, 2), (8, 2)][..], &[8, 13, 15][..]),
    ];
    for (placements, want) in cases {
      let mut room =
        Room { spots: &spots, placed: Vec::new(), left: 8, right: 60 };
      let got: Vec<_> = placements
        .iter()
        .map(|&(link, width)| room.place(link, width))
        .collect();
      assert_eq!(got, want, "{placements:?}");
    }
    // The pane's edges hold labels.
    let edges = [((8, 5), 20), ((19, 6), 19)];
    let mut room =
      Room { spots: &edges, placed: Vec::new(), left: 8, right: 20 };
    assert_eq!(room.place(0, 2), 8, "left edge");
    assert_eq!(room.place(1, 2), 17, "left of the link");
  }

  #[test]
  fn labels_share_length_and_stay_unique() {
    assert_eq!(labels(3), ["a", "s", "d"]);
    let many = labels(ALPHABET.len() + 1);
    assert!(many.iter().all(|label| label.len() == 2));
    assert_eq!(many[0], "aa");
    let unique: std::collections::HashSet<_> = many.iter().collect();
    assert_eq!(unique.len(), many.len());
  }
}
