use super::{Control, Pointing};
use crate::split::Rect;
use unicode_width::UnicodeWidthStr;
use vpdf_model::geometry::At;
use vpdf_model::scene::{Draw, Op};
use vpdf_model::style::{Badge, Role};
use vpdf_model::text::{clean, clipped};

pub(super) struct Button<'a> {
  pub text: &'a str,
  pub control: Control,
}

pub(super) struct Header<'a> {
  pub title: &'a str,
  pub count: usize,
  /// Text after the count; an accent role overrides `role`.
  pub context: &'a [(String, Option<Role>)],
  pub role: Role,
  pub controls: &'a [Button<'a>],
}

impl Header<'_> {
  pub fn scene<'a>(
    &self,
    scene: &mut impl Draw<'a>,
    at: At,
    width: u16,
    pointing: Pointing,
  ) -> Vec<(Rect, Control)> {
    let inner = usize::from(width.saturating_sub(1));
    let controls = if width >= 22 { self.controls } else { &[] };
    let reserved = if controls.is_empty() {
      2
    } else {
      2 + controls.iter().map(|button| button.text.width() + 1).sum::<usize>()
    };
    let columns = inner.saturating_sub(reserved);
    let count = format!(" {}", self.count);
    let title = clean(
      &format!(" {} ", self.title),
      columns.saturating_sub(count.width()),
    );
    let count = clean(&count, columns - title.width());
    let mut room = columns - title.width() - count.width();
    crate::paint::move_to(scene, at, 0u16, 0u16);
    scene.push(Op::Style(Role::Badge(Badge::Cursor)));
    scene.text(&title);
    scene.push(Op::Style(self.role));
    scene.text(&count);
    for (at, (text, accent)) in self.context.iter().enumerate() {
      // Later spans keep their room; this one clips first.
      let later: usize =
        self.context[at + 1..].iter().map(|(text, _)| text.width()).sum();
      let text = clipped(text, room.saturating_sub(later)).0;
      room -= text.width();
      scene.extend(accent.iter().map(|&role| Op::Style(role)));
      scene.text(&text);
      scene.push(Op::Style(self.role));
    }
    scene.push(Op::Spaces(room));
    let mut targets = Vec::new();
    let mut column = columns + 1;
    if controls.is_empty() {
      scene.push(Op::Spaces(inner - columns));
    } else {
      scene.text(" ");
      for button in controls {
        scene.push(Op::Style(
          if pointing == Pointing::Control(button.control) {
            Role::BarHover
          } else {
            self.role
          },
        ));
        scene.push(Op::Style(Role::Heading));
        scene.text(button.text);
        scene.push(Op::Style(self.role));
        scene.text(" ");
        targets.push((
          Rect {
            column: column as u16,
            row: 0,
            columns: button.text.width() as u16,
            rows: 1,
          },
          button.control,
        ));
        column += button.text.width() + 1;
      }
      scene.text(" ");
    }
    scene.push(Op::Style(Role::Reset));
    targets
  }
}

pub(super) fn tip(control: Control, auto: bool) -> Option<&'static str> {
  Some(match control {
    Control::ExpandAll => "Expand all",
    Control::CollapseAll => "Collapse all",
    Control::AutoExpand if auto => "Auto-expand: on",
    Control::AutoExpand => "Auto-expand: off",
    Control::Branch(_) => return None,
  })
}

pub(super) fn fold_controls(auto: bool) -> [Button<'static>; 3] {
  [
    Button { text: "▾▾", control: Control::ExpandAll },
    Button { text: "▸▸", control: Control::CollapseAll },
    Button {
      text: if auto { "●" } else { "○" }, control: Control::AutoExpand
    },
  ]
}
