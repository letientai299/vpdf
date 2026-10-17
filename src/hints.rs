use crate::Result;
use crate::app::App;
use crate::document::Target;
use crate::keys;
use crate::text::{Bounds, PageId};
use crossterm::event::{KeyCode, KeyEvent};
use std::io::Write;

/// Home-row first, like Vimium's default hint characters.
const ALPHABET: &[u8] = b"asdfjklghqweruiopzxcvnm";

pub struct Hint {
  pub label: String,
  pub target: Target,
  /// Top-left cell, zero-based.
  pub cell: (u16, u16),
}

/// Link labels on screen; typing one follows its link.
pub struct Hints {
  pub items: Vec<Hint>,
  pub typed: String,
  /// `F`: copy URIs instead of opening them.
  pub copy: bool,
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

fn label_column(hint: &Hint, left: u16) -> u16 {
  let width = u16::try_from(hint.label.len()).unwrap_or(u16::MAX);
  hint.cell.0.saturating_sub(width).max(left)
}

impl App {
  /// Labels links in view; `copy` copies URIs instead of opening them.
  pub fn start_hints(&mut self, copy: bool) -> Result<()> {
    let mut found = Vec::new();
    let pages = if self.text_mode {
      let page = self.navigation_position().page.0;
      page..page + 1
    } else {
      self.layout.visible(self.reader.scroll)
    };
    for page in pages {
      for link in self.doc.links(PageId(page))? {
        if let Some(cell) = self.link_cell(PageId(page), link.bounds)? {
          found.push((cell, link.target));
        }
      }
    }
    anyhow::ensure!(!found.is_empty(), "No links in view");
    found.sort_by_key(|((column, row), _)| (*row, *column));
    let items = labels(found.len())
      .into_iter()
      .zip(found)
      .map(|(label, (cell, target))| Hint { label, target, cell })
      .collect();
    self.hints = Some(Hints { items, typed: String::new(), copy });
    Ok(())
  }

  fn link_cell(
    &mut self,
    page: PageId,
    bounds: Bounds,
  ) -> Result<Option<(u16, u16)>> {
    if self.text_mode {
      let position = self.doc.text(page)?.nearest(page, bounds.x0, bounds.y0);
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
    let row = rect.y0.max(0.0) as u32 / u32::from(screen.cell_height);
    Ok(Some((self.left + u16::try_from(column)?, u16::try_from(row)?)))
  }

  /// Narrows by typed characters and follows a unique match.
  pub fn hint_key(&mut self, key: KeyEvent) -> Result<()> {
    let Some(hints) = &mut self.hints else { return Ok(()) };
    let key = keys::normalize(key);
    if key.code == KeyCode::Esc {
      self.hints = None;
      self.reader.cancel();
      self.disarm();
      return Ok(());
    }
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
    let mut matches =
      hints.items.iter().filter(|hint| hint.label.starts_with(&hints.typed));
    let Some(first) = matches.next() else {
      let typed = std::mem::take(&mut hints.typed);
      self.hints = None;
      anyhow::bail!("No link labeled {typed}");
    };
    if first.label != hints.typed || matches.next().is_some() {
      return Ok(());
    }
    let (target, copy) = (first.target.clone(), hints.copy);
    self.hints = None;
    match target {
      Target::Uri(uri) if copy => {
        self.reader.store('"', uri)?;
        self.reader.message = "Copied link".into();
        Ok(())
      }
      target => self.follow(&target),
    }
  }

  /// Draws labels; Kitty gets a backdrop image under each label's text.
  pub fn draw_hints(&mut self, out: &mut impl Write) -> Result<()> {
    let Some(hints) = &self.hints else { return Ok(()) };
    let screen = self.layout.screen;
    let (cell_width, cell_height) =
      (f32::from(screen.cell_width), f32::from(screen.cell_height));
    let mut backdrops = Vec::new();
    for hint in &hints.items {
      if !hint.label.starts_with(&hints.typed) {
        continue;
      }
      let column = label_column(hint, self.left);
      let row = hint.cell.1;
      let rest = &hint.label[hints.typed.len()..];
      write!(
        out,
        "{}\x1b[0;1;30;43m{}\x1b[0;30;43m{rest}\x1b[0m",
        self.at.goto(row, column),
        hints.typed
      )?;
      let x = f32::from(column - self.left) * cell_width;
      let y = f32::from(row) * cell_height;
      let width = hint.label.len() as f32 * cell_width;
      backdrops.push(Bounds {
        x0: x,
        y0: y,
        x1: x + width,
        y1: y + cell_height,
      });
    }
    if self.text_mode {
      return Ok(());
    }
    for rect in backdrops {
      self.graphics.backdrop(
        rect,
        [255, 205, 60, 255],
        screen,
        self.left,
        out,
      )?;
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn labels_stay_left_inside_pane() {
    for (link, left, label, expected) in [
      (12, 0, "a", 11),
      (12, 0, "as", 10),
      (12, 8, "as", 10),
      (9, 8, "as", 8),
      (8, 8, "as", 8),
      (0, 0, "as", 0),
    ] {
      let hint = Hint {
        label: label.into(),
        target: Target::Page(PageId(0)),
        cell: (link, 2),
      };
      assert_eq!(label_column(&hint, left), expected, "{link} {left} {label}");
    }
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
