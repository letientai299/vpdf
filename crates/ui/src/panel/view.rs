use super::{
  Action, Destination, Input, Layout, MIN_WIDTH, Motion, MouseButton,
  MouseEvent, MouseEventKind, Panel, PanelKind, Pointing, Row, RowSearch,
  Screen, UnicodeWidthStr, WHEEL_ROWS, Wheel, key_width, moved_row,
  screen_step,
};
use vpdf_model::scene::{Draw, Op, Surface};
use vpdf_model::style::Role;

/// Reader geometry beside or behind a panel.
pub fn page_space(
  screen: Screen,
  panel: Option<&Panel>,
  dragged: Option<u16>,
) -> (u16, Screen) {
  let covered = panel.is_some_and(Panel::covers_page);
  let beside = match panel {
    Some(panel) if !covered => {
      Panel::width(panel.kind(), screen.columns, dragged)
    }
    _ => 0,
  };
  let left = if covered { screen.columns } else { beside };
  (left, Screen { columns: screen.columns.saturating_sub(beside), ..screen })
}

#[derive(Clone, Copy)]
pub(super) enum Navigation {
  Vertical(bool),
  Edge(bool),
  Scroll(f32),
  Screen(i32),
  Rows(i32),
  Heading(bool),
  Align(f32),
  Result(bool),
}

impl Navigation {
  pub(super) fn of(action: Action) -> Option<Self> {
    Some(match action {
      Action::Motion(Motion::Down) => Self::Vertical(true),
      Action::Motion(Motion::Up) => Self::Vertical(false),
      Action::Motion(Motion::First) => Self::Edge(false),
      Action::Motion(Motion::Last) => Self::Edge(true),
      Action::Scroll(amount) => Self::Scroll(amount),
      Action::Screen(direction) => Self::Screen(direction),
      Action::Rows(rows) => Self::Rows(rows),
      Action::Motion(Motion::NextParagraph) => Self::Heading(true),
      Action::Motion(Motion::PreviousParagraph) => Self::Heading(false),
      Action::Align(fraction) => Self::Align(fraction),
      Action::Result(reverse) => Self::Result(reverse),
      _ => return None,
    })
  }

  /// Rows a scroll moves; `page` rows per screen.
  fn rows(self, count: usize, page: usize) -> isize {
    let screen = screen_step(page.saturating_sub(2));
    match self {
      Self::Scroll(amount) => (amount * page as f32 * count as f32) as isize,
      Self::Screen(direction) => {
        direction as isize * screen as isize * count as isize
      }
      Self::Rows(rows) => rows as isize * count as isize,
      _ => 0,
    }
  }
}

impl Panel {
  /// `index`'s line on screen, as scrolled.
  pub(crate) fn screen_line(&self, index: usize) -> isize {
    self.line_start(index) as isize - self.top as isize
  }

  pub fn new(kind: PanelKind, rows: Vec<Row>) -> Self {
    Self {
      kind,
      outline: None,
      projection: super::tree::Projection::all(rows.len()),
      rows,
      selected: 0,
      focused: true,
      top: 0,
      search: RowSearch::default(),
      note: "",
      columns: u16::MAX,
      height: 0,
      layout: None,
      follow_pending: false,
      shelf: None,
      recent: super::selection::Selection::default(),
      picker: crate::home::Picker::Terminal { fzf: false },
      pointing: Pointing::Away,
      hover_title: None,
      clicked: None,
    }
  }

  pub fn recent(rows: Vec<Row>, entries: &[super::recent::Entry]) -> Self {
    Self {
      shelf: Some(super::Shelf::new(entries)),
      ..Self::new(PanelKind::Recent, rows)
    }
  }

  pub fn refresh_recent(
    &mut self,
    rows: Vec<Row>,
    entries: &[super::recent::Entry],
  ) {
    if self.shelf.as_mut().is_some_and(|shelf| shelf.refresh(entries)) {
      self
        .recent
        .checked
        .retain(|path| entries.iter().any(|entry| entry.path == *path));
      self.refill(rows, "");
    }
  }

  pub fn kind(&self) -> PanelKind {
    self.kind
  }
  pub fn rows(&self) -> &[Row] {
    &self.rows
  }
  pub fn selected(&self) -> usize {
    self.selected
  }
  pub fn focused(&self) -> bool {
    self.focused
  }
  pub fn top(&self) -> usize {
    self.top
  }
  pub fn query(&self) -> &str {
    &self.search.query
  }
  pub fn note(&self) -> &'static str {
    self.note
  }
  pub fn hover_title(&self) -> Option<&str> {
    self.hover_title.as_deref()
  }
  pub fn shelf(&self) -> Option<&super::Shelf> {
    self.shelf.as_ref()
  }
  pub fn shelf_mut(&mut self) -> Option<&mut super::Shelf> {
    self.shelf.as_mut()
  }

  pub fn focus(&mut self, focused: bool) {
    if !focused {
      self.leave_recent();
    }
    self.focused = focused;
  }
  pub fn set_note(&mut self, note: &'static str) {
    self.note = note;
  }
  pub fn resize(&mut self, height: u16) {
    self.height = height;
  }
  pub fn point(&mut self, pointing: Pointing) {
    self.pointing = pointing;
  }

  /// Chooses without moving the viewport.
  pub fn choose(&mut self, index: usize) {
    self.selected = index.min(self.rows.len().saturating_sub(1));
  }

  /// # Panics
  /// Mismatched source counts.
  pub fn project(&mut self, projection: super::tree::Projection) {
    assert_eq!(projection.source_count(), self.rows.len());
    let source = self.line_source(self.top);
    self.projection = projection;
    self.selected = self.visible_source(self.visible_position());
    self.clicked = self.clicked.filter(|&source| self.is_visible(source));
    if let Some(source) = self.pointing.row()
      && !self.is_visible(source)
    {
      self.pointing = Pointing::Away;
    }
    self.hover_title = None;
    if let Some(outline) = &mut self.outline {
      outline.targets.clear();
    }
    self.layout = None;
    self.top = self.line_start(source);
  }

  fn replace_outline(&mut self, rows: Vec<Row>, note: &'static str) {
    let projection = super::tree::Projection::all(rows.len());
    self.rows = rows;
    self.outline = None;
    self.selected = 0;
    self.top = 0;
    self.search = RowSearch::default();
    self.clicked = None;
    self.pointing = Pointing::Away;
    self.follow_pending = false;
    self.note = note;
    self.project(projection);
  }

  /// Follows sections after panel browsing ends.
  pub fn track(&mut self, section: Option<usize>, changed: bool) {
    self.follow_pending |= changed;
    if !self.focused
      && self.follow_pending
      && let Some(index) = section
    {
      self.select(index);
      self.follow_pending = false;
    }
  }

  /// Replaces the rows and note, keeping focus and the selected row,
  /// else the selection in range; whether that row stayed.
  pub fn refill(&mut self, rows: Vec<Row>, note: &'static str) -> bool {
    if self.kind == PanelKind::Outline {
      self.replace_outline(rows, note);
      return false;
    }
    let last = rows.len().saturating_sub(1);
    let find = |index: usize| moved_row(&self.rows, index, &rows);
    let before = self.selected;
    let kept = find(before);
    self.selected = kept.unwrap_or(before).min(last);
    // Rows, as the cleared layout reads it; the kept row keeps its line.
    let top = self.line_source(self.top);
    self.top = (top + self.selected).saturating_sub(before);
    if self.top >= rows.len() {
      self.top = 0;
    }
    self.search.follow(|origin| find(origin).unwrap_or(origin).min(last));
    self.clicked = self.clicked.and_then(find);
    if let Pointing::Row(index) = self.pointing {
      self.pointing = find(index).map_or(Pointing::Away, Pointing::Row);
    }
    self.layout = None;
    self.projection = super::tree::Projection::all(rows.len());
    self.rows = rows;
    self.note = note;
    kept.is_some()
  }

  /// Applies list motions; `page` rows per full-screen scroll.
  pub fn navigate(&mut self, input: Input, page: usize) -> bool {
    let Some(navigation) = Navigation::of(input.action) else { return false };
    let last = self.projection.sources().len().saturating_sub(1);
    let position = self.visible_position();
    let count = input.count;
    self.selected = match navigation {
      Navigation::Vertical(true) => {
        self.visible_source(position.saturating_add(count).min(last))
      }
      Navigation::Vertical(false) => {
        self.visible_source(position.saturating_sub(count))
      }
      Navigation::Edge(_) if input.explicit => {
        self.visible_source(count.saturating_sub(1).min(last))
      }
      Navigation::Edge(false) => self.visible_source(0),
      Navigation::Edge(true) => self.visible_source(last),
      Navigation::Scroll(_) | Navigation::Screen(_) | Navigation::Rows(_) => {
        let rows = navigation.rows(count, page);
        if matches!(self.kind, PanelKind::Version) && self.layout.is_some() {
          self.scroll_lines(rows, page);
          return true;
        }
        self.visible_source(position.saturating_add_signed(rows).min(last))
      }
      Navigation::Heading(forward) => {
        self.heading(forward, count);
        self.selected
      }
      Navigation::Align(fraction) => {
        let visible = page.saturating_sub(3) as f32;
        self.top = self
          .line_start(self.selected)
          .saturating_sub((visible * fraction) as usize);
        self.selected
      }
      Navigation::Result(reverse) => {
        self.repeat(reverse, count);
        self.selected
      }
    };
    self.height = u16::try_from(page).unwrap_or(u16::MAX);
    self.reveal();
    true
  }

  /// Returns a clicked row; only presses focus.
  pub fn pointer(&mut self, mouse: MouseEvent) -> Option<usize> {
    if matches!(mouse.kind, MouseEventKind::Down(_)) {
      self.focused = true;
    }
    let card = self.shelf.as_mut().filter(|shelf| shelf.covers(mouse.column));
    if let (Some(shelf), Some(turn)) = (card, Wheel::of(mouse.kind)) {
      let rows = isize::from(WHEEL_ROWS);
      match turn {
        Wheel::Down => shelf.scroll(rows),
        Wheel::Up => shelf.scroll(-rows),
        _ => {}
      }
      return None;
    }
    match mouse.kind {
      kind if Wheel::of(kind) == Some(Wheel::Down) => {
        let visible = usize::from(self.height.saturating_sub(2));
        self.top = self
          .top
          .saturating_add(usize::from(WHEEL_ROWS))
          .min(self.line_count().saturating_sub(visible.max(1)));
      }
      kind if Wheel::of(kind) == Some(Wheel::Up) => {
        self.top = self.top.saturating_sub(usize::from(WHEEL_ROWS));
      }
      MouseEventKind::Down(MouseButton::Left) => {
        let index = self.row_at(mouse.column, mouse.row)?;
        self.selected = index;
        return Some(index);
      }
      _ => {}
    }
    None
  }

  pub fn covers_page(&self) -> bool {
    self.is_recent_view() && self.focused()
  }

  /// Recent rows open with Enter alone.
  pub fn is_recent_view(&self) -> bool {
    self.kind == PanelKind::Recent
  }

  /// What `yy` copies from the selected row: its label.
  pub fn yank_text(&self) -> Option<String> {
    let row = self.rows.get(self.selected)?;
    Some(format!("{}\n", row.label.trim()))
  }

  /// The row at a panel cell.
  pub fn row_at(&self, column: u16, row: u16) -> Option<usize> {
    if row == 0 || column + 1 >= self.columns {
      return None;
    }
    if self.outline.is_some()
      && usize::from(self.columns) <= super::outline::GUTTER
    {
      return None;
    }
    if (self.layout.is_some() || self.outline.is_some())
      && row >= self.height.saturating_sub(1)
    {
      return None;
    }
    let line = self.top + usize::from(row) - 1;
    if line >= self.line_count() {
      return None;
    }
    Some(self.line_source(line)).filter(|&index| index < self.rows.len())
  }

  /// Selects row `index` and shows it.
  pub fn select(&mut self, index: usize) {
    self.selected = index;
    self.reveal();
  }

  pub fn reveal(&mut self) {
    let count = usize::from(self.height.saturating_sub(2));
    let start = self.line_start(self.selected);
    let end = self.line_start(self.selected + 1);
    if start < self.top || end.saturating_sub(start) > count {
      self.top = start;
    } else if count > 0 && end > self.top + count {
      self.top = end - count;
    }
  }

  fn scroll_lines(&mut self, rows: isize, page: usize) {
    self.height = u16::try_from(page).unwrap_or(u16::MAX);
    let visible = page.saturating_sub(2).max(1);
    let offset =
      self.line_start(self.selected).saturating_sub(self.top).min(visible - 1);
    self.top = self
      .top
      .saturating_add_signed(rows)
      .min(self.line_count().saturating_sub(visible));
    self.selected = self.line_source(self.top + offset);
  }

  fn line_count(&self) -> usize {
    self
      .layout
      .as_ref()
      .map_or(self.projection.sources().len(), |help| help.lines.len())
  }

  fn line_source(&self, line: usize) -> usize {
    self.layout.as_ref().map_or_else(
      || self.projection.source(line).unwrap_or(self.rows.len()),
      |help| help.lines.get(line).map_or(self.rows.len(), |line| line.source),
    )
  }

  pub(super) fn line_start(&self, row: usize) -> usize {
    self.layout.as_ref().map_or_else(
      || self.projection.sources().partition_point(|&source| source < row),
      |help| help.starts.get(row).copied().unwrap_or(help.lines.len()),
    )
  }

  pub(super) fn is_visible(&self, source: usize) -> bool {
    self.projection.position(source).is_some()
  }

  fn visible_position(&self) -> usize {
    self.projection.position(self.selected).unwrap_or_else(|| {
      self
        .projection
        .sources()
        .partition_point(|&row| row < self.selected)
        .saturating_sub(1)
    })
  }

  fn visible_source(&self, position: usize) -> usize {
    self.projection.source(position).unwrap_or(0)
  }

  pub(super) fn reflow(&mut self, columns: usize) {
    if matches!(self.kind, PanelKind::Version | PanelKind::Outline)
      && self.layout.as_ref().is_some_and(|layout| layout.columns == columns)
      || !matches!(
        self.kind,
        PanelKind::Version | PanelKind::Jumps | PanelKind::Outline
      ) && self.projection.sources().len() == self.rows.len()
    {
      return;
    }
    let source = self.line_source(self.top);
    let offset = self.top.saturating_sub(self.line_start(source));
    let selected = self.line_start(self.selected);
    let visible = usize::from(self.height.saturating_sub(2));
    let shown = (self.top..self.top + visible).contains(&selected);
    self.layout = Some(
      match self.kind {
        PanelKind::Jumps => Layout::jumps(&self.rows, columns, self.selected),
        PanelKind::Version => Layout::help(&self.rows, columns),
        _ => Layout::plain(&self.rows, columns),
      }
      .project(&self.projection),
    );
    let start = self.line_start(source);
    let length = self.line_start(source + 1).saturating_sub(start);
    self.top = start + offset.min(length.saturating_sub(1));
    if shown {
      self.reveal();
    }
  }

  /// Panel columns: dragged, else by kind.
  pub fn width(kind: PanelKind, columns: u16, dragged: Option<u16>) -> u16 {
    let wide = matches!(kind, PanelKind::Info | PanelKind::Version);
    let width = dragged.map_or_else(
      || {
        let width = if wide { columns / 2 } else { columns / 3 };
        width.clamp(1, if wide { 64 } else { 50 })
      },
      |dragged| dragged.max(MIN_WIDTH),
    );
    width.min(columns.saturating_sub(12))
  }

  pub fn scene<'a, S: Surface<'a>>(
    &mut self,
    scene: &mut S,
    frame: crate::panel::recent::Frame,
    pictures: bool,
  ) -> Result<(), S::Error> {
    let crate::panel::recent::Frame { width, screen, at, .. } = frame;
    if self.is_recent_view() && self.rows.is_empty() {
      crate::home::welcome::scene(scene, screen, at, self.picker);
      self.scene_confirmation(scene, at, width, screen.rows);
      return Ok(());
    }
    let Some(mut cards) = self.shelf.take() else {
      self.scene_list(scene, at, width, screen.rows);
      self.scene_confirmation(scene, at, width, screen.rows);
      return Ok(());
    };
    if width < screen.columns || !self.focused {
      cards.hide();
      self.shelf = Some(cards);
      self.scene_list(scene, at, width, screen.rows);
      self.scene_confirmation(scene, at, width, screen.rows);
      return Ok(());
    }
    let drawn = cards.scene(self, scene, frame, pictures);
    self.shelf = Some(cards);
    self.scene_confirmation(scene, at, width, screen.rows);
    drawn
  }

  pub(super) fn row_roles(&self, source: usize) -> (Role, Role) {
    let selected = self.kind != PanelKind::Version && source == self.selected;
    let plain = match (selected, self.focused) {
      (true, true) => Role::Selected,
      (true, false) => Role::Current,
      (false, true) => Role::Reset,
      (false, false) => Role::Dim,
    };
    let hovered = self.pointing.row() == Some(source);
    let follows = self.rows[source].follows();
    let base = match (hovered && !selected && follows, self.focused) {
      (true, true) => Role::Hover,
      (true, false) => Role::HoverDim,
      (false, _) => plain,
    };
    (plain, base)
  }

  pub fn scene_list<'a>(
    &mut self,
    scene: &mut impl Draw<'a>,
    at: vpdf_model::geometry::At,
    width: u16,
    height: u16,
  ) {
    self.columns = width;
    self.height = height;
    self.hover_title = None;
    if let Some(outline) = &mut self.outline {
      outline.targets.clear();
    }
    if width < 2 || height == 0 {
      return;
    }
    let inner = usize::from(width.saturating_sub(1));
    for row in 1..height {
      crate::paint::move_to(scene, at, row - 1, width - 1);
      scene.push(Op::Style(Role::Muted));
      scene.text("│");
      scene.push(Op::Style(Role::Reset));
    }
    if self.outline.is_some() {
      self.scene_outline(scene, at, width, height);
      return;
    }
    let header_room = inner.saturating_sub(2);
    let selection = if self.is_recent_view() {
      self.selection_count()
    } else {
      String::new()
    };
    let context = format!(" {}{selection}", self.note);
    super::header::Header {
      title: self.kind.title(),
      count: self.rows.len(),
      context: &context,
      role: Role::Muted,
    }
    .scene(scene, at, header_room);
    scene.push(Op::Spaces(inner - header_room));
    scene.push(Op::Style(Role::Reset));
    self.reflow(inner);
    let keys = key_width(&self.rows, inner);
    let count = height.saturating_sub(2) as usize;
    self.top = self.top.min(self.line_count().saturating_sub(count.max(1)));
    let start = self.top;
    for (row, index) in (start..self.line_count()).take(count).enumerate() {
      let (source, item, keys, cut) = self.layout.as_ref().map_or_else(
        || (index, &self.rows[index], keys, None),
        |help| {
          let line = &help.lines[index];
          // The row's label line, even scrolled off.
          let label = &help.lines[help.starts[line.source]];
          (line.source, &line.row, line.keys, Some(label.clipped))
        },
      );
      let hovered = self.pointing.row() == Some(source);
      let (plain, base) = self.row_roles(source);
      crate::paint::move_to(scene, at, row + 1, 0u16);
      scene.push(Op::Style(base));
      let checkbox = if self.is_recent_view()
        && matches!(item.destination, Destination::File(_))
      {
        let checked = matches!(&item.destination, Destination::File(path)
          if self.recent.checked.contains(path));
        if checked { "[x] " } else { "[ ] " }
      } else {
        ""
      };
      let gutter = checkbox.len().min(inner);
      scene.text(&checkbox[..gutter]);
      let available = inner.saturating_sub(gutter);
      let detail = vpdf_model::text::clean(&item.detail, available / 3);
      let room = if detail.is_empty() {
        available
      } else {
        available.saturating_sub(detail.width() + 1)
      };
      let used = item.scene(scene, Some(base), room, keys, &self.search.query);
      if hovered {
        let clipped = match cut {
          Some(clipped) => clipped,
          None => item.scene(&mut (), Some(base), usize::MAX, keys, "") > used,
        };
        if clipped {
          // Outline rows indent by depth.
          self.hover_title = Some(self.rows[source].label.trim().to_owned());
        }
      }
      scene.extend([
        Op::Style(plain),
        Op::Spaces(available.saturating_sub(used + detail.width())),
      ]);
      if plain == Role::Reset {
        scene.push(Op::Style(Role::Muted));
      }
      scene.text(&detail);
      scene.push(Op::Style(Role::Reset));
    }
  }
}
