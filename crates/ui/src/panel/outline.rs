use super::{Panel, Pointing, tree::Tree};
use crate::keys::{Action, Fold};
use crate::split::Rect;
use unicode_width::UnicodeWidthStr;
use vpdf_model::geometry::At;
use vpdf_model::scene::{Draw, Op};
use vpdf_model::style::Role;
use vpdf_model::text::{clean, clipped};

pub(super) const GUTTER: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Control {
  Branch(usize),
  ExpandAll,
  CollapseAll,
  AutoExpand,
}

struct Branch {
  depth: usize,
  root: bool,
  descendants: usize,
  closed: bool,
}

pub(super) struct OutlineView {
  branches: Vec<Branch>,
  current: Option<usize>,
  auto: bool,
  pub(super) targets: Vec<(Rect, Control)>,
}

struct RowLayout {
  indent: usize,
  title: String,
  clipped: bool,
  badge: String,
  page: String,
  padding: usize,
}

impl RowLayout {
  fn new(branch: &Branch, row: &super::Row, width: u16) -> Self {
    let inner = usize::from(width.saturating_sub(1));
    let available = inner.saturating_sub(GUTTER);
    let label = clean(&row.detail, available);
    let page = if label.width() == row.detail.width()
      && label.width() + 2 <= available
      && !label.is_empty()
    {
      format!(" {label}")
    } else {
      String::new()
    };
    let badge = if width >= 28 && branch.closed && branch.descendants > 0 {
      format!(" +{}", branch.descendants)
    } else {
      String::new()
    };
    let badge = if badge.width() + page.width() + 4 <= available {
      badge
    } else {
      String::new()
    };
    let room = available.saturating_sub(page.width() + badge.width());
    let indent =
      branch.depth.min(12).saturating_mul(2).min(room.saturating_sub(4));
    let (title, cut) = clipped(row.label.trim_start(), room - indent);
    Self {
      indent,
      padding: room.saturating_sub(indent + title.width()),
      title,
      clipped: cut,
      badge,
      page,
    }
  }
}

impl Panel {
  pub fn decorate_outline(
    &mut self,
    tree: &Tree,
    closed: impl Fn(usize) -> bool,
    current: Option<usize>,
    auto: bool,
  ) {
    self.outline = Some(OutlineView {
      branches: tree
        .nodes()
        .iter()
        .enumerate()
        .map(|(source, node)| Branch {
          depth: node.depth,
          root: node.parent.is_none(),
          descendants: node.descendants,
          closed: closed(source),
        })
        .collect(),
      current,
      auto,
      targets: Vec::new(),
    });
  }

  pub fn mark_outline(&mut self, tree: &Tree, current: Option<usize>) {
    if let Some(outline) = &mut self.outline {
      outline.current = current
        .and_then(|source| tree.visible_ancestor(source, &self.projection));
    }
  }

  pub fn auto_expand(&self) -> bool {
    self.outline.as_ref().is_some_and(|outline| outline.auto)
  }

  pub fn control_at(&self, column: u16, row: u16) -> Option<Control> {
    self.outline.as_ref()?.targets.iter().find_map(|(rect, control)| {
      rect.contains(column, row).then_some(*control)
    })
  }

  pub fn control_action(&self, control: Control) -> Option<Action> {
    Some(match control {
      Control::Branch(source) => {
        if !self.is_visible(source) {
          return None;
        }
        let branch = self.outline.as_ref()?.branches.get(source)?;
        Action::Fold(if branch.closed { Fold::Open } else { Fold::Close })
      }
      Control::ExpandAll => Action::Fold(Fold::ExpandAll),
      Control::CollapseAll => Action::Fold(Fold::CollapseAll),
      Control::AutoExpand => Action::OutlineAuto,
    })
  }

  pub fn control_tip(&self, control: Control) -> Option<String> {
    let action = self.control_action(control)?;
    let label = match control {
      Control::Branch(_) if matches!(action, Action::Fold(Fold::Open)) => {
        "Expand branch"
      }
      Control::Branch(_) => "Collapse branch",
      Control::ExpandAll => "Expand all",
      Control::CollapseAll => "Collapse all",
      Control::AutoExpand if self.auto_expand() => "Auto-expand: on",
      Control::AutoExpand => "Auto-expand: off",
    };
    Some(label.into())
  }

  fn outline_header<'a>(
    &self,
    scene: &mut impl Draw<'a>,
    at: At,
    width: u16,
  ) -> Vec<(Rect, Control)> {
    let inner = usize::from(width.saturating_sub(1));
    let outline = self.outline.as_ref().expect("outline decoration");
    let mut targets = Vec::new();
    let auto = if outline.auto { "●" } else { "○" };
    let controls = if width >= 22 { 9 + auto.width() } else { 0 };
    let header_room = inner.saturating_sub(controls.max(2));
    let count_label = format!(" {}", self.rows.len());
    let title_room = header_room.saturating_sub(count_label.width());
    let title = clean(" CONTENTS ", title_room);
    crate::paint::move_to(scene, at, 0u16, 0u16);
    scene.push(Op::Style(Role::Badge(super::Color::Cyan)));
    scene.text(&title);
    scene.push(Op::Style(Role::Bar));
    let count_label = clean(&count_label, header_room - title.width());
    scene.text(&count_label);
    let note_room =
      header_room.saturating_sub(title.width() + count_label.width());
    let note = clean(&format!(" {}", self.note), note_room);
    scene.text(&note);
    scene.push(Op::Spaces(note_room - note.width()));
    if controls > 0 {
      scene.text(" ");
      for (offset, text, control) in [
        (0, "▾▾", Control::ExpandAll),
        (3, "▸▸", Control::CollapseAll),
        (6, auto, Control::AutoExpand),
      ] {
        let hovered = self.pointing == Pointing::Control(control);
        scene.push(Op::Style(if hovered { Role::BarHover } else { Role::Bar }));
        scene.push(Op::Style(Role::Heading));
        scene.text(text);
        scene.push(Op::Style(Role::Bar));
        scene.text(" ");
        targets.push((
          Rect {
            column: (header_room + 1 + offset) as u16,
            row: 0,
            columns: text.width() as u16,
            rows: 1,
          },
          control,
        ));
      }
      scene.text(" ");
    } else {
      scene.push(Op::Spaces(inner - header_room));
    }
    scene.push(Op::Style(Role::Reset));
    targets
  }

  pub(super) fn scene_outline<'a>(
    &mut self,
    scene: &mut impl Draw<'a>,
    at: At,
    width: u16,
    height: u16,
  ) {
    let inner = usize::from(width.saturating_sub(1));
    self.reflow(inner);
    let count = height.saturating_sub(2) as usize;
    self.top = self
      .top
      .min(self.projection.sources().len().saturating_sub(count.max(1)));
    let outline = self.outline.as_ref().expect("outline decoration");
    let mut targets = self.outline_header(scene, at, width);
    for (line, &source) in
      self.projection.sources().iter().skip(self.top).take(count).enumerate()
    {
      if inner < GUTTER {
        break;
      }
      let branch = &outline.branches[source];
      let item = &self.rows[source];
      let layout = RowLayout::new(branch, item, width);
      let hovered = self.pointing.row() == Some(source);
      let (base, title_style) = self.row_roles(source);
      crate::paint::move_to(scene, at, line + 1, 0u16);
      scene.push(Op::Style(base));
      if outline.current == Some(source) {
        scene.push(Op::Style(Role::Heading));
        scene.text("▌");
        scene.push(Op::Style(base));
      } else {
        scene.text(" ");
      }
      scene.push(Op::Spaces(layout.indent));
      scene.push(Op::Style(Role::Heading));
      scene.text(if branch.descendants == 0 {
        "  "
      } else if branch.closed {
        "▸ "
      } else {
        "▾ "
      });
      if branch.descendants > 0 {
        targets.push((
          Rect {
            column: (1 + layout.indent) as u16,
            row: line as u16 + 1,
            columns: 2,
            rows: 1,
          },
          Control::Branch(source),
        ));
      }
      super::rows::marked(
        scene,
        &layout.title,
        &self.search.query,
        Some(title_style),
        branch.root.then_some(Role::Heading),
      );
      if hovered && layout.clipped {
        self.hover_title = Some(item.label.trim().to_owned());
      }
      scene.push(Op::Style(base));
      scene.push(Op::Spaces(layout.padding));
      scene.push(Op::Style(Role::Muted));
      scene.text(&layout.badge);
      scene.push(Op::Style(base));
      super::rows::marked(
        scene,
        &layout.page,
        &self.search.query,
        Some(base),
        None,
      );
      scene.push(Op::Style(Role::Reset));
    }
    self.outline.as_mut().expect("outline decoration").targets = targets;
  }
}
