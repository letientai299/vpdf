use super::{Destination, Event, Panel};
use crate::split::Rect;
use std::collections::BTreeSet;
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;
use vpdf_model::input::{
  KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
  MouseEventKind,
};
use vpdf_model::scene::{Draw, Op};
use vpdf_model::style::Role;

#[derive(Default)]
pub(super) struct Selection {
  pub checked: BTreeSet<PathBuf>,
  confirmation: Option<Confirmation>,
}

struct Confirmation {
  removal: Removal,
  /// Button cells from the last frame, panel-relative; true confirms.
  buttons: Vec<(Rect, bool)>,
}

enum Removal {
  Files {
    first: PathBuf,
    rest: Vec<PathBuf>,
  },
  /// A mark, and the file a global one points at.
  Mark {
    name: char,
    file: Option<String>,
  },
}

impl Removal {
  /// The question and the consequence to confirm.
  fn text(&self) -> (String, Option<&'static str>) {
    match self {
      Self::Files { first, rest } if rest.is_empty() => (
        format!(
          "Remove \"{}\" from recent?",
          first.file_name().unwrap_or_default().to_string_lossy()
        ),
        Some("Saved position and local marks will be forgotten."),
      ),
      Self::Files { rest, .. } => (
        format!("Remove {} files from recent?", rest.len() + 1),
        Some("Saved position and local marks will be forgotten."),
      ),
      Self::Mark { name, file: None } => {
        (format!("Remove mark ’{name}?"), None)
      }
      Self::Mark { name, file: Some(file) } => (
        format!("Remove global mark ’{name} to {file}?"),
        Some("Every document loses it."),
      ),
    }
  }

  fn event(self) -> Event {
    match self {
      Self::Files { first, rest } => {
        Event::Forget(std::iter::once(first).chain(rest).collect())
      }
      Self::Mark { name, .. } => Event::RemoveMark(name),
    }
  }
}

const BUTTONS: [(&str, bool); 2] =
  [("[ Remove Y/Enter ]", true), ("[ Cancel n/Esc ]", false)];

impl Panel {
  pub fn confirming(&self) -> bool {
    self.removal.confirmation.is_some()
  }

  pub fn leave_removal(&mut self) {
    self.removal = Selection::default();
  }

  pub fn clear_checked(&mut self) -> bool {
    let changed = !self.removal.checked.is_empty();
    self.removal.checked.clear();
    changed
  }

  pub(super) fn toggle_checked(&mut self) {
    let Some(Destination::File(path)) =
      self.rows.get(self.selected).map(|row| &row.destination)
    else {
      return;
    };
    if !self.removal.checked.remove(path) {
      self.removal.checked.insert(path.clone());
    }
  }

  pub(super) fn request_removal(&mut self) {
    if !self.is_recent_view() {
      let Some(row) = self.rows.get(self.selected) else { return };
      let Some(name) = row.mark_name() else { return };
      let file = matches!(row.destination, Destination::Mark(_))
        .then(|| row.label.clone());
      let removal = Removal::Mark { name, file };
      self.removal.confirmation =
        Some(Confirmation { removal, buttons: Vec::new() });
      return;
    }
    let targets = if self.removal.checked.is_empty() {
      match self.rows.get(self.selected).map(|row| &row.destination) {
        Some(Destination::File(path)) => vec![path.clone()],
        _ => return,
      }
    } else {
      self.removal.checked.iter().cloned().collect()
    };
    let mut targets = targets.into_iter();
    let Some(first) = targets.next() else { return };
    let removal = Removal::Files { first, rest: targets.collect() };
    self.removal.confirmation =
      Some(Confirmation { removal, buttons: Vec::new() });
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
      return self.answer(false);
    }
    if crate::keys::plain(key)
      && matches!(key.code, KeyCode::Enter | KeyCode::Char('y'))
    {
      return self.answer(true);
    }
    Event::Stay
  }

  /// A press on a confirmation button, in panel cells.
  pub fn confirm_press(&mut self, mouse: MouseEvent) -> Event {
    if !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
      return Event::Stay;
    }
    let Some(confirmation) = &self.removal.confirmation else {
      return Event::Stay;
    };
    let pressed = confirmation.buttons.iter().find_map(|(rect, yes)| {
      rect.contains(mouse.column, mouse.row).then_some(*yes)
    });
    pressed.map_or(Event::Stay, |yes| self.answer(yes))
  }

  fn answer(&mut self, yes: bool) -> Event {
    match self.removal.confirmation.take() {
      Some(confirmation) if yes => confirmation.removal.event(),
      _ => Event::Stay,
    }
  }

  /// The question, its consequence and the buttons, above the last row.
  pub fn scene_confirmation<'a>(
    &mut self,
    scene: &mut impl Draw<'a>,
    at: vpdf_model::geometry::At,
    width: u16,
    height: u16,
  ) {
    let Some(confirmation) = &mut self.removal.confirmation else { return };
    let (question, detail) = confirmation.removal.text();
    confirmation.buttons.clear();
    // Too small for buttons: the question names its keys.
    if height < 3 || usize::from(width) <= BUTTONS[0].0.width() {
      scene.push(Op::Style(Role::Bar));
      let row = height.saturating_sub(2);
      crate::paint::line(scene, at, row, &format!("{question} [Y/n]"), width);
      scene.push(Op::Style(Role::Reset));
      return;
    }
    let lines: Vec<&str> =
      std::iter::once(question.as_str()).chain(detail).collect();
    let buttons = height.saturating_sub(2);
    let first = buttons.saturating_sub(lines.len() as u16);
    scene.push(Op::Style(Role::Bar));
    for (row, text) in (first..buttons).zip(lines) {
      crate::paint::line(scene, at, row, text, width);
    }
    crate::paint::move_to(scene, at, buttons, 0u16);
    let mut column = 0;
    for (text, yes) in BUTTONS {
      let cells = text.width();
      if column + 1 + cells > usize::from(width) {
        break;
      }
      scene.extend([Op::Style(Role::Bar), Op::Spaces(1)]);
      scene.extend([Op::Style(Role::Bar), Op::Style(Role::Heading)]);
      scene.text(text);
      let rect = Rect {
        column: (column + 1) as u16,
        row: buttons,
        columns: cells as u16,
        rows: 1,
      };
      confirmation.buttons.push((rect, yes));
      column += 1 + cells;
    }
    scene.extend([
      Op::Style(Role::Bar),
      Op::Spaces(usize::from(width).saturating_sub(column)),
      Op::Style(Role::Reset),
    ]);
  }

  pub(super) fn selection_count(&self) -> String {
    let count = self.removal.checked.len();
    let shown = self
      .rows
      .iter()
      .filter(|row| {
        matches!(&row.destination, Destination::File(path)
        if self.removal.checked.contains(path))
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
