use super::{Destination, Event, Panel};
use std::collections::BTreeSet;
use std::path::PathBuf;
use vpdf_model::input::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use vpdf_model::scene::Draw;
use vpdf_model::style::Role;

#[derive(Default)]
pub(super) struct Selection {
  pub checked: BTreeSet<PathBuf>,
  confirmation: Option<Confirmation>,
}

struct Confirmation {
  first: PathBuf,
  rest: Vec<PathBuf>,
}

impl Panel {
  pub fn confirming(&self) -> bool {
    self.recent.confirmation.is_some()
  }

  pub fn leave_recent(&mut self) {
    self.recent = Selection::default();
  }

  pub fn clear_checked(&mut self) -> bool {
    let changed = !self.recent.checked.is_empty();
    self.recent.checked.clear();
    changed
  }

  pub(super) fn toggle_checked(&mut self) {
    let Some(Destination::File(path)) =
      self.rows.get(self.selected).map(|row| &row.destination)
    else {
      return;
    };
    if !self.recent.checked.remove(path) {
      self.recent.checked.insert(path.clone());
    }
  }

  pub(super) fn request_removal(&mut self) {
    let targets = if self.recent.checked.is_empty() {
      match self.rows.get(self.selected).map(|row| &row.destination) {
        Some(Destination::File(path)) => vec![path.clone()],
        _ => return,
      }
    } else {
      self.recent.checked.iter().cloned().collect()
    };
    let mut targets = targets.into_iter();
    let Some(first) = targets.next() else { return };
    self.recent.confirmation =
      Some(Confirmation { first, rest: targets.collect() });
  }

  pub fn confirm_key(&mut self, key: KeyEvent) -> Event {
    if key.kind == KeyEventKind::Release || !self.confirming() {
      return Event::Stay;
    }
    let key = crate::keys::normalize(key);
    if key.code == KeyCode::Esc
      || key.code == KeyCode::Char('c')
        && key.modifiers.contains(KeyModifiers::CONTROL)
      || key.code == KeyCode::Char('n') && crate::keys::plain(key)
    {
      self.recent.confirmation = None;
    } else if crate::keys::plain(key)
      && matches!(key.code, KeyCode::Enter | KeyCode::Char('y'))
    {
      let Some(confirmation) = self.recent.confirmation.take() else {
        return Event::Stay;
      };
      return Event::Forget(
        std::iter::once(confirmation.first).chain(confirmation.rest).collect(),
      );
    }
    Event::Stay
  }

  pub fn scene_confirmation<'a>(
    &self,
    scene: &mut impl Draw<'a>,
    at: vpdf_model::geometry::At,
    width: u16,
    height: u16,
  ) {
    let Some(confirmation) = &self.recent.confirmation else { return };
    let question = if confirmation.rest.is_empty() {
      format!(
        "Remove \"{}\" from recent? [Y/n]",
        confirmation.first.file_name().unwrap_or_default().to_string_lossy()
      )
    } else {
      format!("Remove {} files from recent? [Y/n]", confirmation.rest.len() + 1)
    };
    for (index, (row, text)) in [
      (height.saturating_sub(3), question.as_str()),
      (
        height.saturating_sub(2),
        "Saved position and local marks will be forgotten.",
      ),
    ]
    .into_iter()
    .enumerate()
    {
      if index == 1 && height < 3 {
        continue;
      }
      scene.push(vpdf_model::scene::Op::Style(Role::Bar));
      crate::paint::line(scene, at, row, text, width);
    }
    scene.push(vpdf_model::scene::Op::Style(Role::Reset));
  }

  pub(super) fn selection_count(&self) -> String {
    let count = self.recent.checked.len();
    let shown = self
      .rows
      .iter()
      .filter(|row| {
        matches!(&row.destination, Destination::File(path)
        if self.recent.checked.contains(path))
      })
      .count();
    if count > shown {
      format!(" · {count} selected ({} hidden)", count - shown)
    } else {
      format!(" · {count} selected")
    }
  }

  pub fn set_picker(&mut self, picker: crate::home::Picker) {
    self.picker = picker;
  }
}
