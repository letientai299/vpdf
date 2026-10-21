use crate::Result;
use crate::keys;
use crate::layout::Layout;
use crate::paint::Frame;
use crate::pointer::Items;
use crate::split::Rect;
use crate::theme;
use vpdf_model::document::{Link, Target};
use vpdf_model::input::{KeyCode, KeyEvent};
use vpdf_model::scene::{Draw, Layer, Op};
use vpdf_model::style::Role;
use vpdf_model::text::{Bounds, PageId};
#[cfg(test)]
use vpdf_terminal::graphics::Graphics;

#[derive(Debug, PartialEq, Eq)]
pub enum Transition {
  Stay,
  Cancel,
  Close,
  Run(usize),
  Miss(String),
}

enum Route {
  Run(usize),
  Cancel,
  Type(char),
  Erase,
  Close,
  Pass,
}

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
  items: Vec<Hint>,
  typed: String,
  /// `F`: copy URIs instead of opening them.
  copy: bool,
  /// The pointer's label; Enter runs it.
  selected: Option<usize>,
}

/// `count` labels of equal length, so no label prefixes another.
fn labels(count: usize) -> Vec<String> {
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
pub type Spot = ((u16, u16), u16);

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

impl Hints {
  pub fn new(
    mut found: Vec<(Spot, Target)>,
    copy: bool,
    left: u16,
    right: u16,
  ) -> Result<Self> {
    anyhow::ensure!(!found.is_empty(), "No links in view");
    found.sort_by_key(|(((column, row), _), _)| (*row, *column));
    let spots: Vec<_> = found.iter().map(|(spot, _)| *spot).collect();
    let mut room = Room { spots: &spots, placed: Vec::new(), left, right };
    let items = (labels(found.len()).into_iter().zip(found).enumerate())
      .map(|(index, (label, (spot, target)))| {
        let column = room.place(index, width(&label));
        Hint { label, target, cell: spot.0, column }
      })
      .collect();
    let typed = String::new();
    Ok(Self { items, typed, copy, selected: None })
  }

  pub fn items(&self) -> &[Hint] {
    &self.items
  }
  pub fn typed(&self) -> &str {
    &self.typed
  }
  pub fn selected(&self) -> Option<usize> {
    self.selected
  }

  pub fn select(&mut self, index: usize) {
    let shown = self.shown().any(|(shown, _)| shown == index);
    self.selected = shown.then_some(index);
  }

  pub fn into_target(self, index: usize) -> Option<(Target, bool)> {
    self.items.into_iter().nth(index).map(|hint| (hint.target, self.copy))
  }

  fn route(&self, key: KeyEvent) -> Route {
    let key = keys::normalize(key);
    if key.code == KeyCode::Enter
      && let Some(index) = self.selected
    {
      return Route::Run(index);
    }
    if key.code == KeyCode::Esc {
      return Route::Cancel;
    }
    match (keys::plain(key), key.code) {
      (true, KeyCode::Char(ch)) if ch.is_ascii_alphabetic() => {
        Route::Type(ch.to_ascii_lowercase())
      }
      (true, KeyCode::Backspace) if !self.typed.is_empty() => Route::Erase,
      (_, KeyCode::Backspace) => Route::Close,
      _ => Route::Pass,
    }
  }

  /// Keys retained after global cancellation handling.
  pub fn accepts(&self, key: KeyEvent) -> bool {
    !matches!(self.route(key), Route::Cancel | Route::Pass)
  }

  pub fn key(&mut self, key: KeyEvent) -> Transition {
    let route = self.route(key);
    // Typing may hide the picked label.
    if !matches!(route, Route::Run(_) | Route::Cancel) {
      self.selected = None;
    }
    match route {
      Route::Run(index) => return Transition::Run(index),
      Route::Cancel => return Transition::Cancel,
      Route::Type(ch) => self.typed.push(ch),
      Route::Erase => {
        self.typed.pop();
        return Transition::Stay;
      }
      Route::Close | Route::Pass => return Transition::Close,
    }
    let shown: Vec<_> = self.shown().map(|(index, _)| index).take(2).collect();
    match shown[..] {
      [] => {
        let typed = std::mem::take(&mut self.typed);
        Transition::Miss(typed)
      }
      [index] if self.items[index].label == self.typed => {
        Transition::Run(index)
      }
      _ => Transition::Stay,
    }
  }

  pub fn hit_items(&self) -> Items {
    let mut items = Items::default();
    for (index, hint) in self.shown() {
      items.push(hint.cells(), index);
    }
    items
  }

  pub fn scene<'a>(&self, scene: &mut impl Draw<'a>, frame: Frame) {
    let mut backdrops = Vec::new();
    for (index, hint) in self.shown() {
      let picked = self.selected == Some(index);
      let (typed, rest, fill) = if picked {
        (Role::HintPickedTyped, Role::HintPicked, theme::BADGE_FILL)
      } else {
        (Role::HintTyped, Role::Hint, theme::HINT_FILL)
      };
      let cells = hint.cells();
      crate::paint::move_to(scene, frame.at, cells.row, cells.column);
      scene.push(Op::Style(typed));
      scene.text(&self.typed);
      scene.push(Op::Style(rest));
      scene.text(&hint.label[self.typed.len()..]);
      scene.push(Op::Style(Role::Reset));
      backdrops.push((
        crate::paint::cell_bounds(frame.screen, frame.left, cells),
        fill,
      ));
    }
    if frame.text {
      return;
    }
    let screen = frame.screen;
    for (rect, fill) in backdrops {
      scene.fill(rect, fill, Layer::BelowText, screen, frame.left);
    }
  }
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

pub fn spot(
  link: &Link,
  mut cell_at: impl FnMut(Bounds) -> Result<Option<(u16, u16)>>,
) -> Result<Option<Spot>> {
  for bounds in link.boxes() {
    let Some(cell) = cell_at(bounds)? else { continue };
    let edge = Bounds { x0: bounds.x1, ..bounds };
    let end = cell_at(edge)?.map_or(cell.0, |end| end.0);
    return Ok(Some((cell, end.max(cell.0))));
  }
  Ok(None)
}

pub fn page_cell(
  layout: &Layout,
  scroll: f32,
  left: u16,
  page: PageId,
  bounds: Bounds,
) -> Result<Option<(u16, u16)>> {
  let rect = layout.project(page, bounds, scroll);
  let screen = layout.screen();
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
  let middle = f32::midpoint(top, bottom).min((screen.height() - 1.0).max(0.0));
  let row = middle.max(0.0) as u32 / u32::from(screen.cell_height);
  Ok(Some((left + u16::try_from(column)?, u16::try_from(row)?)))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn recorded_labels_keep_backdrops() -> Result<()> {
    use vpdf_model::geometry::At;
    use vpdf_model::geometry::Screen;
    use vpdf_model::scene::Scene;
    let mut hints =
      Hints::new(vec![(((4, 2), 5), Target::Page(PageId(0)))], false, 0, 20)?;
    let screen = Screen {
      columns: 20,
      rows: 10,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    for picked in [false, true] {
      hints.selected = picked.then_some(0);
      for text in [false, true] {
        let frame = Frame { at: At::default(), screen, left: 0, text };
        let mut scene = Scene::new();
        hints.scene(&mut scene, frame);
        let mut actual = Vec::new();
        let mut graphics = Graphics::default();
        let mut painter = vpdf_terminal::painter::Painter::new(&mut actual)
          .with_graphics(&mut graphics);
        painter.paint(&scene);
        painter.finish()?;
        let mut expected = if picked {
          b"\x1b[3;4H\x1b[0;1;97;44m\x1b[0;97;44ma\x1b[0m".to_vec()
        } else {
          b"\x1b[3;4H\x1b[0;1;30;43m\x1b[0;30;43ma\x1b[0m".to_vec()
        };
        let fill = if picked { theme::BADGE_FILL } else { theme::HINT_FILL };
        if !text {
          Graphics::default().backdrop(
            Bounds { x0: 24.0, y0: 32.0, x1: 32.0, y1: 48.0 },
            fill,
            screen,
            0,
            &mut expected,
          )?;
        }
        assert_eq!(actual, expected);
      }
    }
    Ok(())
  }

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

  #[test]
  fn input_returns_effects_and_keeps_selection_valid() -> Result<()> {
    use vpdf_model::input::KeyModifiers;
    let count = u16::try_from(ALPHABET.len()).expect("short alphabet");
    let found = (0..=count)
      .map(|row| (((10, row), 12), Target::Page(PageId(0))))
      .collect();
    let mut hints = Hints::new(found, false, 0, 80)?;
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    hints.select(1);
    assert_eq!(hints.key(key(KeyCode::Enter)), Transition::Run(1));
    assert_eq!(hints.key(key(KeyCode::Char('a'))), Transition::Stay);
    assert_eq!((hints.typed(), hints.selected()), ("a", None));
    hints.select(ALPHABET.len());
    assert_eq!(hints.selected(), None, "hidden label cannot be picked");
    assert_eq!(hints.key(key(KeyCode::Backspace)), Transition::Stay);
    assert_eq!(hints.typed(), "");
    assert_eq!(hints.key(key(KeyCode::Char('A'))), Transition::Stay);
    assert_eq!(hints.key(key(KeyCode::Char('s'))), Transition::Run(1));
    assert_eq!(hints.key(key(KeyCode::Esc)), Transition::Cancel);
    assert_eq!(
      hints.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)),
      Transition::Close
    );
    let mut hints =
      Hints::new(vec![(((10, 0), 12), Target::Page(PageId(0)))], true, 0, 80)?;
    assert_eq!(
      hints.key(key(KeyCode::Char('b'))),
      Transition::Miss("b".into())
    );
    assert_eq!(hints.into_target(0), Some((Target::Page(PageId(0)), true)));
    assert!(Hints::new(Vec::new(), false, 0, 80).is_err());
    Ok(())
  }

  #[test]
  fn hint_gate_preserves_state() -> Result<()> {
    use vpdf_model::input::KeyModifiers;
    for (selected, typed) in
      [(false, false), (false, true), (true, false), (true, true)]
    {
      let found =
        (0..25).map(|row| (((10, row), 12), Target::Page(PageId(0)))).collect();
      let mut hints = Hints::new(found, false, 0, 80)?;
      if typed {
        hints.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
      }
      if selected {
        hints.select(0);
      }
      let before = (hints.typed().to_owned(), hints.selected());
      for (code, modifiers, accepts) in [
        (KeyCode::Char('A'), KeyModifiers::SHIFT, true),
        (KeyCode::Char('a'), KeyModifiers::CONTROL, false),
        (KeyCode::Char('a'), KeyModifiers::ALT, false),
        (KeyCode::Backspace, KeyModifiers::NONE, true),
        (KeyCode::Backspace, KeyModifiers::CONTROL, true),
        (KeyCode::Enter, KeyModifiers::ALT, selected),
        (KeyCode::Char('1'), KeyModifiers::NONE, false),
        (KeyCode::Down, KeyModifiers::NONE, false),
        (KeyCode::Esc, KeyModifiers::NONE, false),
      ] {
        assert_eq!(hints.accepts(KeyEvent::new(code, modifiers)), accepts);
        assert_eq!(
          (hints.typed(), hints.selected()),
          (before.0.as_str(), before.1)
        );
      }
      assert_eq!(
        hints.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL)),
        Transition::Close
      );
    }
    Ok(())
  }
}
