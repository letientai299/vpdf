use super::{
  App, Axis, Edge, Hover, Instant, MouseButton, MouseEvent, MouseEventKind,
  Pixel, Rect, Region, Result, Session,
};

impl Session {
  /// The boundary at a cell.
  pub(super) fn edge_at(&self, column: u16, row: u16) -> Option<Edge> {
    let tab = self.tabs.get(self.active)?;
    let mut dividers = Vec::new();
    tab.tree.dividers(self.area(), &mut dividers);
    let on = |axis| {
      let mut found = dividers.iter().filter(|divider| divider.axis == axis);
      found.find(|divider| divider.rect.contains(column, row)).copied()
    };
    if let Some(divider) = on(Axis::Columns) {
      return Some(Edge::Split(divider));
    }
    // The panel's last column, beside the page.
    let border = tab
      .panes
      .iter()
      .filter_map(super::Pane::document)
      .map(App::pointer_pane)
      .find(|pane| {
        let panel = pane.area(Region::Panel);
        let edge = Rect {
          column: pane.rect.column + panel.columns.saturating_sub(2),
          row: pane.rect.row,
          columns: panel.columns.min(2),
          rows: panel.rows,
        };
        (1..pane.rect.columns).contains(&pane.panel)
          && edge.contains(column, row)
      });
    if let Some(pane) = border {
      return Some(Edge::Panel(pane.id));
    }
    // Status items stay clickable.
    let item = || {
      tab.panes.iter().any(|pane| {
        let rect = pane.pointer_pane().rect;
        rect.contains(column, row)
          && pane.chrome_at(column - rect.column, row - rect.row).is_some()
      })
    };
    on(Axis::Rows).filter(|_| !item()).map(Edge::Split)
  }

  /// Boundary drags and releases; whether drawn.
  pub(super) fn edge_event(
    &mut self,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) -> Result<Option<bool>> {
    let Some(edge) = self.resizing else { return Ok(None) };
    let release = match mouse.kind {
      MouseEventKind::Drag(MouseButton::Left) => false,
      MouseEventKind::Up(MouseButton::Left) => true,
      _ => return Ok(None),
    };
    // Ends the capture; keeps the spot.
    self.pointer.route(&[], 0, mouse, pixel, now);
    if release {
      self.resizing = None;
      return Ok(Some(true));
    }
    // Moves past the threshold, as pane drags.
    if !self.pointer.dragged() {
      return Ok(Some(false));
    }
    self.drag_edge(edge, mouse).map(Some)
  }

  /// A draggable boundary, as a hover.
  pub(super) fn edge_hover(&mut self, mouse: MouseEvent) -> Option<Hover> {
    // Overlays and held presses take the press.
    let free = !self.tabs.is_empty()
      && !self.pointer.holding()
      && self.focused().document().is_none_or(|app| !app.overlaid());
    let edge = self.edge_at(mouse.column, mouse.row).filter(|_| free)?;
    let pane = match edge {
      Edge::Split(_) => 0,
      Edge::Panel(id) => id,
    };
    let region = Region::Edge;
    Some(Hover { pane, region, hit: edge.hit(), beneath: None })
  }

  /// Starts a drag; double clicks equalize.
  pub(super) fn edge_press(
    &mut self,
    edge: Edge,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) -> Result<bool> {
    let clicks = self.pointer.press_edge(mouse, pixel, now);
    self.resizing = Some(edge);
    if clicks < 2 {
      return Ok(true);
    }
    let area = self.area();
    match edge {
      Edge::Split(divider) => self.tab().tree.equalize_at(divider.index, area),
      Edge::Panel(id) => {
        if let Some(index) = self.tab().index(id) {
          let Some(app) = self.tab().panes[index].document_mut() else {
            return Ok(true);
          };
          app.panel_width = None;
          app.relayout()?;
        }
      }
    }
    self.arrange()?;
    Ok(true)
  }

  /// Moves the dragged boundary to the pointer.
  fn drag_edge(&mut self, edge: Edge, mouse: MouseEvent) -> Result<bool> {
    let area = self.area();
    match edge {
      Edge::Split(divider) => {
        let at = match divider.axis {
          Axis::Columns => mouse.column,
          Axis::Rows => mouse.row,
        };
        let tree = &mut self.tab().tree;
        let mut before = Vec::new();
        tree.layout(area, &mut before);
        tree.drag(divider.index, at, area);
        let mut after = Vec::new();
        tree.layout(area, &mut after);
        if before == after {
          return Ok(false);
        }
        self.arrange()?;
      }
      Edge::Panel(id) => {
        let Some(index) = self.tab().index(id) else { return Ok(false) };
        let Some(app) = self.tab().panes[index].document_mut() else {
          return Ok(false);
        };
        return app.resize_panel(
          mouse.column.saturating_sub(app.at.column).saturating_add(1),
        );
      }
    }
    Ok(true)
  }
}
