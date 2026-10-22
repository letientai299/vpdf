use crate::grid::{self, Grid};
use crate::keys;
use crate::layout::Screen;
use crate::pointer::Items;
use crate::text::Position;
use std::time::{Duration, Instant};
use vpdf_model::geometry::At;
use vpdf_model::input::KeyEvent;

#[derive(Default)]
pub struct Clues {
  state: State,
}

#[derive(Default)]
enum State {
  #[default]
  Hidden,
  Waiting(Instant),
  Shown(List),
}

pub struct List {
  items: Vec<(String, String)>,
  labels: Vec<String>,
  events: Vec<Option<Vec<KeyEvent>>>,
  keys: usize,
  top: usize,
}

pub struct Clue {
  label: String,
  about: String,
  sequence: Option<crate::bindings::Sequence>,
}

impl Clues {
  pub const DELAY: Duration = Duration::from_millis(500);

  pub fn restart(&mut self, since: Option<Instant>) {
    self.state = since.map_or(State::Hidden, State::Waiting);
  }

  pub fn pending(&self) -> bool {
    matches!(self.state, State::Waiting(_))
  }

  pub fn wait(&self, now: Instant) -> Option<Duration> {
    let State::Waiting(since) = self.state else { return None };
    Some(Self::DELAY.saturating_sub(now.saturating_duration_since(since)))
  }

  pub fn poll(&mut self, now: Instant) -> bool {
    if self.wait(now) != Some(Duration::ZERO) {
      return false;
    }
    self.state = State::Hidden;
    true
  }

  pub fn show(&mut self, items: Vec<(String, String)>) -> bool {
    let events = items.iter().map(|(key, _)| clue_keys(key)).collect();
    self.show_with(items, events)
  }

  pub fn show_items(&mut self, items: Vec<Clue>) -> bool {
    let events = items
      .iter()
      .map(|item| {
        item.sequence.as_ref().map(|sequence| sequence.keys().to_vec())
      })
      .collect();
    let items =
      items.into_iter().map(|item| (item.label, item.about)).collect();
    self.show_with(items, events)
  }

  fn show_with(
    &mut self,
    items: Vec<(String, String)>,
    events: Vec<Option<Vec<KeyEvent>>>,
  ) -> bool {
    self.state = State::Hidden;
    if items.is_empty() {
      return false;
    }
    let keys = items.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
    let labels = items
      .iter()
      .map(|(key, about)| format!("{key:<keys$}  {about}"))
      .collect();
    self.state = State::Shown(List { items, labels, events, keys, top: 0 });
    true
  }

  pub fn shown(&self) -> Option<&List> {
    match &self.state {
      State::Shown(list) => Some(list),
      _ => None,
    }
  }

  pub fn scroll(&mut self, rows: isize, screen: Screen) {
    let State::Shown(list) = &mut self.state else { return };
    let grid = list.grid(screen);
    list.top =
      grid.first.saturating_add_signed(rows).min(grid.rows - grid.visible);
  }

  pub fn rows(&self, screen: Screen) -> u16 {
    self
      .shown()
      .map_or(0, |list| u16::try_from(list.grid(screen).visible).unwrap_or(0))
  }
}

impl List {
  pub fn items(&self) -> &[(String, String)] {
    &self.items
  }
  pub fn labels(&self) -> &[String] {
    &self.labels
  }
  pub fn top(&self) -> usize {
    self.top
  }

  pub fn keys(&self, index: usize) -> Option<Vec<KeyEvent>> {
    self.events.get(index).cloned().flatten()
  }

  pub fn grid(&self, screen: Screen) -> Grid {
    let columns = usize::from(screen.columns);
    let limit = grid::limit(screen);
    let mut grid = grid::layout(&self.labels, columns, limit, None);
    grid.first = self.top.min(grid.rows.saturating_sub(grid.visible));
    grid
  }

  pub fn cells(&self, screen: Screen, top: u16) -> Items {
    let grid = self.grid(screen);
    let cells = grid.cells(self.labels.len(), top);
    cells.filter(|(_, index)| self.keys(*index).is_some()).collect()
  }

  pub fn scene<'a>(
    &self,
    scene: &mut impl vpdf_model::scene::Draw<'a>,
    bottom: (At, u16),
    screen: Screen,
    pointed: Option<usize>,
  ) {
    grid::scene(
      scene,
      bottom,
      &self.labels,
      &self.grid(screen),
      (pointed, None),
      self.keys,
      screen,
    );
  }
}

pub fn items<'a>(
  keys: &keys::Keys,
  grammar: keys::Grammar,
  allows: impl Fn(keys::Action) -> bool,
  registers: impl Iterator<Item = (char, &'a str)>,
  marks: impl Iterator<Item = (char, Position)>,
) -> Vec<(String, String)> {
  let Some(_) = keys.awaiting() else { return Vec::new() };
  let owned = |(key, about): (&str, &str)| (key.to_owned(), about.to_owned());
  match keys.choosing() {
    Some(crate::bindings::Control::Register) => register_clues(registers),
    Some(crate::bindings::Control::SetMark)
      if allows(keys::Action::Mark('a')) =>
    {
      [
        ("a-z", "mark here in this document"),
        ("A-Z", "mark here across documents"),
      ]
      .map(owned)
      .to_vec()
    }
    Some(crate::bindings::Control::GoMark { exact })
      if allows(keys::Action::GoMark('a', exact)) =>
    {
      let mut items: Vec<_> = marks
        .map(|(name, at)| (name.to_string(), format!("page {}", at.page.0 + 1)))
        .collect();
      items.push(("A-Z".into(), "mark in any document".into()));
      if let Some(key) = keys.key_for(keys::Action::History(false)) {
        items.push((key, "back to the last jump".into()));
      }
      items
    }
    _ => keys
      .clues(grammar, allows)
      .into_iter()
      .map(|(key, about)| (key.replace(' ', "Space"), about.to_owned()))
      .collect(),
  }
}

pub fn effective_items<'a>(
  keys: &keys::Keys,
  grammar: keys::Grammar,
  allows: impl Fn(keys::Action) -> bool,
  registers: impl Iterator<Item = (char, &'a str)>,
  marks: impl Iterator<Item = (char, Position)>,
) -> Vec<Clue> {
  if keys.choosing().is_some() {
    return items(keys, grammar, allows, registers, marks)
      .into_iter()
      .map(|(label, about)| {
        let sequence = if ["a-z", "A-Z", "+ *"].contains(&label.as_str()) {
          None
        } else {
          crate::bindings::Sequence::parse(&label).ok()
        };
        Clue { label, about, sequence }
      })
      .collect();
  }
  keys
    .effective_clues(allows)
    .into_iter()
    .map(|(sequence, about)| Clue {
      label: sequence.to_string(),
      about,
      sequence: Some(sequence),
    })
    .collect()
}

fn register_clues<'a>(
  registers: impl Iterator<Item = (char, &'a str)>,
) -> Vec<(String, String)> {
  let mut items: Vec<_> = registers
    .map(|(name, text)| {
      let preview: String = text
        .split_whitespace()
        .flat_map(|word| std::iter::once(' ').chain(word.chars()))
        .skip(1)
        .take(40)
        .collect();
      (name.to_string(), preview)
    })
    .collect();
  items.sort();
  items.push(("+ *".into(), "system clipboard".into()));
  items.push(("a-z".into(), "named register; A-Z appends".into()));
  items
}

fn clue_keys(label: &str) -> Option<Vec<KeyEvent>> {
  let range = label.len() == 3
    && label.as_bytes()[1] == b'-'
    && label.as_bytes()[0] != b'C';
  if range || label.contains(' ') {
    return None;
  }
  if label.contains('<') {
    return crate::bindings::Sequence::parse(label)
      .ok()
      .map(|sequence| sequence.keys().to_vec())
      .or_else(|| keys::parse(label));
  }
  keys::parse(label)
}

#[cfg(test)]
mod tests;
