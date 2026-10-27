use super::layout::{Meta, key_cells};
use super::{Panel, Style, tree::Tree};
use crate::keys::{Action, Fold, PanelKind};
use crate::split::Rect;
use unicode_width::UnicodeWidthStr;
use vpdf_model::geometry::At;
use vpdf_model::scene::{Draw, Op};
use vpdf_model::style::Role;
use vpdf_model::text::clipped;

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

impl Branch {
  /// Headings skip the key column; `+N` needs 28 cells.
  fn meta(&self, row: &super::Row, width: u16, keys: usize) -> Meta {
    let hidden = if width >= 28 && self.closed { self.descendants } else { 0 };
    let keys = if self.descendants > 0 { 0 } else { keys };
    let available = usize::from(width.saturating_sub(1)).saturating_sub(GUTTER);
    Meta::new(row, available, keys, self.depth, hidden)
  }
}

pub(super) struct TreeView {
  branches: Vec<Branch>,
  auto: Option<bool>,
  pub(super) targets: Vec<(Rect, Control)>,
}

impl Panel {
  pub fn decorate_tree(
    &mut self,
    tree: &Tree,
    closed: impl Fn(usize) -> bool,
    auto: Option<bool>,
  ) {
    self.tree_view = Some(TreeView {
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
      auto,
      targets: Vec::new(),
    });
  }

  pub fn branch_closed(&self, source: usize) -> Option<bool> {
    self
      .tree_view
      .as_ref()?
      .branches
      .get(source)
      .filter(|branch| branch.descendants > 0)
      .map(|branch| branch.closed)
  }

  pub(crate) fn row_control(&self, source: usize) -> Option<Control> {
    (!self.rows.get(source)?.follows() && self.branch_closed(source).is_some())
      .then_some(Control::Branch(source))
  }

  pub fn auto_expand(&self) -> bool {
    self.tree_view.as_ref().is_some_and(|tree| tree.auto == Some(true))
  }

  pub fn control_at(&self, column: u16, row: u16) -> Option<Control> {
    self.tree_view.as_ref()?.targets.iter().find_map(|(rect, control)| {
      rect.contains(column, row).then_some(*control)
    })
  }

  pub fn control_action(&self, control: Control) -> Option<Action> {
    Some(match control {
      Control::Branch(source) => {
        if !self.is_visible(source) {
          return None;
        }
        let branch = self.tree_view.as_ref()?.branches.get(source)?;
        Action::Fold(if branch.closed { Fold::Open } else { Fold::Close })
      }
      Control::ExpandAll => Action::Fold(Fold::ExpandAll),
      Control::CollapseAll => Action::Fold(Fold::CollapseAll),
      Control::AutoExpand => {
        self.tree_view.as_ref()?.auto?;
        Action::OutlineAuto
      }
    })
  }

  pub fn control_tip(&self, control: Control) -> Option<String> {
    let action = self.control_action(control)?;
    let label = match control {
      Control::Branch(_) if matches!(action, Action::Fold(Fold::Open)) => {
        if self.kind == PanelKind::Outline {
          "Expand branch"
        } else {
          "Expand group"
        }
      }
      Control::Branch(_) => {
        if self.kind == PanelKind::Outline {
          "Collapse branch"
        } else {
          "Collapse group"
        }
      }
      _ => super::header::tip(control, self.auto_expand())?,
    };
    Some(label.into())
  }

  /// The header with its fold controls; their click targets.
  fn scene_tree_header<'a>(
    &self,
    scene: &mut impl Draw<'a>,
    at: At,
    width: u16,
  ) -> Vec<(Rect, Control)> {
    let auto = self.tree_view.as_ref().and_then(|tree| tree.auto);
    let controls = super::header::fold_controls(auto == Some(true));
    let shown = match auto {
      Some(_) => 3,
      // Empty groups have nothing to fold.
      None if self.rows.is_empty() => 0,
      None => 2,
    };
    super::header::Header {
      title: self.kind.title(),
      count: self.row_count(),
      context: &self.context(),
      role: Role::Bar,
      controls: &controls[..shown],
    }
    .scene(scene, at, width, self.pointing)
  }

  pub(super) fn scene_tree<'a>(
    &mut self,
    scene: &mut impl Draw<'a>,
    at: At,
    width: u16,
    height: u16,
  ) {
    let inner = usize::from(width.saturating_sub(1));
    let count = height.saturating_sub(2) as usize;
    match self.kind {
      // Laid-out rows indent by depth as `Branch::meta` does.
      PanelKind::Annotations => self.reflow(inner.saturating_sub(GUTTER)),
      // Help lines sit under group headings, two cells past the gutter.
      PanelKind::Version => self.reflow(inner.saturating_sub(GUTTER + 2)),
      _ => {}
    }
    self.top = self.top.min(self.line_count().saturating_sub(count.max(1)));
    let mut targets = self.scene_tree_header(scene, at, width);
    let tree = self.tree_view.as_ref().expect("tree decoration");
    let keys = key_cells(&self.rows, inner.saturating_sub(GUTTER));
    let keys_width = super::key_width(&self.rows, inner.saturating_sub(GUTTER));
    if count > 0 {
      self.scene_empty(scene, at, inner);
    }
    let current = self.current_row();
    let lines = self.top..self.line_count();
    for (line, index) in lines.take(count).enumerate() {
      if inner < GUTTER {
        break;
      }
      let source = self.line_source(index);
      let branch = &tree.branches[source];
      let laid = self.layout.as_ref().map(|layout| &layout.lines[index]);
      let item = &self.rows[source];
      let meta = branch.meta(item, width, keys);
      if let Some(extra) = laid.filter(|_| self.line_start(source) != index) {
        let (base, _) = self.row_roles(source);
        crate::paint::move_to(scene, at, line + 1, 0u16);
        // Laid-out lines carry their own lead.
        let indent = GUTTER + meta.indent;
        let query = &self.search.query;
        scene_extra(scene, &extra.row, query, base, (indent, inner));
        continue;
      }
      let hovered = self.pointing.row() == Some(source);
      let (base, title_style) = self.row_roles(source);
      crate::paint::move_to(scene, at, line + 1, 0u16);
      scene.push(Op::Style(base));
      super::rows::gutter(scene, current == Some(source), base);
      scene.push(Op::Spaces(meta.indent));
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
            column: (1 + meta.indent) as u16,
            row: line as u16 + 1,
            columns: 2,
            rows: 1,
          },
          Control::Branch(source),
        ));
      }
      meta.lead(scene, base, &self.search.query);
      // A laid-out row shows its first wrapped line.
      let shown = laid.map_or(item, |line| &line.row);
      let (used, cut) = if matches!(shown.style, Style::Plain | Style::Heading)
      {
        let (title, cut) = clipped(shown.label.trim_start(), meta.room);
        let cut = cut || laid.is_some_and(|line| line.clipped);
        super::rows::marked(
          scene,
          &title,
          &self.search.query,
          Some(title_style),
          branch.root.then_some(Role::Heading),
        );
        (title.width(), cut)
      } else {
        let query = &self.search.query;
        let keys = laid.map_or(keys_width, |line| line.keys);
        let used =
          shown.scene(scene, Some(title_style), meta.room, keys, query);
        let cut =
          laid.map_or(item.label.width() > meta.room, |line| line.clipped);
        (used, cut)
      };
      if hovered && cut {
        self.hover_title = Some(item.full_label());
      }
      scene.push(Op::Style(base));
      scene.push(Op::Spaces(meta.room - used));
      scene.push(Op::Style(super::rows::quiet(base)));
      scene.text(&meta.badge);
      scene.push(Op::Style(base));
      super::rows::marked(
        scene,
        &meta.page,
        &self.search.query,
        Some(base),
        None,
      );
      scene.push(Op::Style(Role::Reset));
    }
    self.tree_view.as_mut().expect("tree decoration").targets = targets;
  }
}

/// A row's wrapped or note line, `indent` cells in, marking `query`.
fn scene_extra<'a>(
  scene: &mut impl Draw<'a>,
  row: &super::Row,
  query: &str,
  base: Role,
  (indent, inner): (usize, usize),
) {
  let room = inner.saturating_sub(indent);
  scene.extend([Op::Style(base), Op::Spaces(inner.min(indent))]);
  let used = row.scene(scene, Some(base), room, 0, query);
  scene.extend([
    Op::Style(base),
    Op::Spaces(room - used),
    Op::Style(Role::Reset),
  ]);
}
