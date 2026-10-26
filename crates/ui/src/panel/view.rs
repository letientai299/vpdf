use super::layout::{Meta, key_cells};
use super::{
  Action, Destination, Input, Layout, MIN_WIDTH, Motion, MouseButton,
  MouseEvent, MouseEventKind, Panel, PanelKind, Pointing, Row, RowSearch,
  Screen, WHEEL_ROWS, Wheel, key_width, moved_row, screen_step,
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
      tree_view: None,
      folds: None,
      projection: super::tree::Projection::all(rows.len()),
      rows,
      selected: 0,
      focused: true,
      top: 0,
      search: RowSearch::default(),
      note: "",
      topic: String::new(),
      current: None,
      columns: u16::MAX,
      height: 0,
      layout: None,
      follow_pending: false,
      new_topic: false,
      place: String::new(),
      shelf: None,
      removal: super::selection::Selection::default(),
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
        .removal
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
      self.leave_removal();
    }
    self.focused = focused;
  }
  pub fn set_note(&mut self, note: &'static str) {
    self.note = note;
  }
  /// A new topic opens every group.
  pub fn set_topic(&mut self, topic: &str) {
    if self.topic == topic {
      return;
    }
    topic.clone_into(&mut self.topic);
    self.new_topic = true;
    if let Some(mut folds) = self.folds.take() {
      folds.expand_all();
      folds.project(self, None);
      self.folds = Some(folds);
    }
  }

  /// Rows the header counts; group headings are not counted.
  pub(super) fn row_count(&self) -> usize {
    if self.folds.is_none() {
      return self.rows.len();
    }
    self.rows.iter().filter(|row| row.depth > 0).count()
  }

  pub fn set_place(&mut self, place: &str) {
    if self.place != place {
      place.clone_into(&mut self.place);
    }
  }

  /// The selected hit's number among the results, from 1.
  fn item_index(&self) -> Option<usize> {
    let row = self.rows.get(self.selected)?;
    if self.kind != PanelKind::Results || row.depth == 0 {
      return None;
    }
    let before = &self.rows[..self.selected];
    Some(before.iter().filter(|row| row.depth > 0).count() + 1)
  }

  /// Header text after the count: the topic, the selected hit or mark,
  /// the place, the note and the selection.
  pub(super) fn context(&self) -> Vec<(String, Option<Role>)> {
    let mut context = Vec::new();
    if !self.topic.is_empty() {
      context.push((format!(" “{}”", self.topic), Some(Role::Key)));
    }
    if let Some(index) = self.item_index() {
      context.push((format!("  {index}/{}", self.row_count()), None));
    }
    if let Some(name) = self.rows.get(self.selected).and_then(Row::mark_name) {
      context.push((format!("  ’{name} jump"), None));
    }
    if !self.place.is_empty() {
      context.push((format!("  {}", self.place), None));
    }
    let selection = if self.is_recent_view() {
      self.selection_count()
    } else {
      String::new()
    };
    context.push((format!(" {}{selection}", self.note), None));
    context
  }

  pub fn current(&self) -> Option<usize> {
    self.current
  }

  /// Marks the row that lands on `landed`.
  pub fn mark_landed(&mut self, landed: Option<crate::text::Position>) {
    self.current = landed.and_then(|landed| {
      self.rows.iter().position(|row| {
        matches!(row.destination, Destination::Position(at) if at == landed)
      })
    });
  }

  /// Marks the row at the reading position; never the selection.
  pub fn mark_current(&mut self, source: Option<usize>) {
    self.current = source;
  }

  /// The visible row holding the current one: itself, else the folded
  /// heading over it. Projections come only from `Tree::project`, which
  /// hides whole preorder subtrees, so the last shown row at or before
  /// the current one is its shown ancestor.
  pub(super) fn current_row(&self) -> Option<usize> {
    let current = self.current.filter(|&source| source < self.rows.len())?;
    let sources = self.projection.sources();
    let shown = sources.partition_point(|&source| source <= current);
    sources.get(shown.checked_sub(1)?).copied()
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
    if let Some(tree) = &mut self.tree_view {
      tree.targets.clear();
    }
    self.layout = None;
    self.top = self.line_start(source);
  }

  fn replace_outline(&mut self, rows: Vec<Row>, note: &'static str) {
    let projection = super::tree::Projection::all(rows.len());
    self.rows = rows;
    self.tree_view = None;
    self.selected = 0;
    self.top = 0;
    self.search = RowSearch::default();
    self.clicked = None;
    self.current = None;
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
    self.current = self.current.and_then(find);
    if let Pointing::Row(index) = self.pointing {
      self.pointing = find(index).map_or(Pointing::Away, Pointing::Row);
    }
    self.layout = None;
    // Groups describe the old rows.
    self.folds = None;
    self.tree_view = None;
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
    Some(format!("{}\n", row.full_label()))
  }

  /// The row at a panel cell.
  pub fn row_at(&self, column: u16, row: u16) -> Option<usize> {
    if row == 0 || column + 1 >= self.columns {
      return None;
    }
    if self.tree_view.is_some()
      && usize::from(self.columns) <= super::outline::GUTTER
    {
      return None;
    }
    if (self.layout.is_some() || self.tree_view.is_some())
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

  /// Panel row of the selection's first line, when shown.
  pub fn selected_row(&self) -> Option<u16> {
    let line = self.line_start(self.selected).checked_sub(self.top)?;
    let row = u16::try_from(line + 1).ok()?;
    (self.row_at(0, row) == Some(self.selected)).then_some(row)
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

  pub(super) fn line_count(&self) -> usize {
    self
      .layout
      .as_ref()
      .map_or(self.projection.sources().len(), |help| help.lines.len())
  }

  pub(super) fn line_source(&self, line: usize) -> usize {
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

  pub fn is_visible(&self, source: usize) -> bool {
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
    let fresh = self.layout.as_ref().is_some_and(|layout| {
      layout.columns == columns
        && layout.selected.is_none_or(|selected| selected == self.selected)
    });
    let moved = self
      .layout
      .as_ref()
      .and_then(|layout| layout.selected)
      .is_some_and(|selected| selected != self.selected);
    let partial = self.projection.sources().len() != self.rows.len();
    let build: fn(&[Row], usize, usize) -> Layout = match self.kind {
      PanelKind::Jumps => Layout::jumps,
      PanelKind::Annotations => Layout::annotations,
      PanelKind::Version => |rows, columns, _| Layout::help(rows, columns),
      // A projected list keeps the cut its hover reads.
      _ if partial => |rows, columns, _| Layout::plain(rows, columns),
      // Other lists paint their rows directly.
      _ => return,
    };
    if fresh {
      return;
    }
    let source = self.line_source(self.top);
    let offset = self.top.saturating_sub(self.line_start(source));
    let selected = self.line_start(self.selected);
    let visible = usize::from(self.height.saturating_sub(2));
    let shown = (self.top..self.top + visible).contains(&selected);
    // A new selection keeps its screen line as other rows collapse.
    let anchor = selected.checked_sub(self.top).filter(|_| moved && shown);
    self.layout =
      Some(build(&self.rows, columns, self.selected).project(&self.projection));
    let start = self.line_start(source);
    let length = self.line_start(source + 1).saturating_sub(start);
    self.top = match anchor {
      Some(line) => self.line_start(self.selected).saturating_sub(line),
      None => start + offset.min(length.saturating_sub(1)),
    };
    if shown {
      self.reveal();
    }
  }

  /// Panel columns: dragged, else by kind.
  pub fn width(kind: PanelKind, columns: u16, dragged: Option<u16>) -> u16 {
    let (share, most) = match kind {
      PanelKind::Info => (2, 50),
      PanelKind::Version => (2, 64),
      _ => (3, 50),
    };
    let width = dragged.map_or_else(
      || (columns / share).clamp(1, most),
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
    // The flat help view has no selection to show.
    let flat = self.kind == PanelKind::Version && self.folds.is_none();
    let selected = !flat && source == self.selected;
    let plain = match (selected, self.focused) {
      (true, true) => Role::Selected,
      (true, false) => Role::Current,
      (false, true) => Role::Reset,
      (false, false) => Role::Dim,
    };
    let hovered = self.pointing.row() == Some(source);
    let actionable =
      self.rows[source].follows() || self.row_control(source).is_some();
    let base = match (hovered && !selected && actionable, self.focused) {
      (true, true) => Role::Hover,
      (true, false) => Role::HoverDim,
      (false, _) => plain,
    };
    (plain, base)
  }

  fn scene_heading<'a>(
    &self,
    scene: &mut impl Draw<'a>,
    at: vpdf_model::geometry::At,
    width: u16,
  ) {
    super::header::Header {
      title: self.kind.title(),
      count: self.row_count(),
      context: &self.context(),
      role: Role::Bar,
      controls: &[],
    }
    .scene(scene, at, width, self.pointing);
    scene.push(Op::Style(Role::Reset));
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
    if let Some(tree) = &mut self.tree_view {
      tree.targets.clear();
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
    if self.tree_view.is_some() {
      self.scene_tree(scene, at, width, height);
      return;
    }
    self.scene_heading(scene, at, width);
    let count = height.saturating_sub(2) as usize;
    if count > 0 {
      self.scene_empty(scene, at, inner);
    }
    let marker = usize::from(self.kind.marks_current()).min(inner);
    let current = self.current_row();
    let inner = inner - marker;
    self.reflow(inner);
    let keys = key_width(&self.rows, inner);
    let key_column = key_cells(&self.rows, inner);
    self.top = self.top.min(self.line_count().saturating_sub(count.max(1)));
    let start = self.top;
    for (row, index) in (start..self.line_count()).take(count).enumerate() {
      let (source, item, keys, cut, first) = self.layout.as_ref().map_or_else(
        || (index, &self.rows[index], keys, None, true),
        |help| {
          let line = &help.lines[index];
          // The row's label line, even scrolled off.
          let start = help.starts[line.source];
          let label = &help.lines[start];
          (
            line.source,
            &line.row,
            line.keys,
            Some(label.clipped),
            index == start,
          )
        },
      );
      let hovered = self.pointing.row() == Some(source);
      let (plain, base) = self.row_roles(source);
      crate::paint::move_to(scene, at, row + 1, 0u16);
      scene.push(Op::Style(base));
      if marker > 0 {
        let shown = first || row == 0;
        super::rows::gutter(scene, shown && current == Some(source), base);
      }
      let checkbox = if self.is_recent_view()
        && matches!(item.destination, Destination::File(_))
      {
        let checked = matches!(&item.destination, Destination::File(path)
          if self.removal.checked.contains(path));
        if checked { "[x] " } else { "[ ] " }
      } else {
        ""
      };
      let gutter = checkbox.len().min(inner);
      scene.text(&checkbox[..gutter]);
      // Only a row's first line reserves the key column.
      let key_column = if first { key_column } else { 0 };
      let meta =
        Meta::new(item, inner.saturating_sub(gutter), key_column, 0, 0);
      meta.lead(scene, plain, &self.search.query);
      let used =
        item.scene(scene, Some(base), meta.room, keys, &self.search.query);
      if hovered {
        let clipped = match cut {
          Some(clipped) => clipped,
          None => item.scene(&mut (), Some(base), usize::MAX, keys, "") > used,
        };
        if clipped {
          self.hover_title = Some(self.rows[source].full_label());
        }
      }
      let detail = meta.page.trim_start();
      let gap = usize::from(!detail.is_empty());
      scene.extend([
        Op::Style(plain),
        Op::Spaces((meta.room + gap).saturating_sub(used)),
      ]);
      if plain == Role::Reset {
        scene.push(Op::Style(Role::Muted));
      }
      scene.text(detail);
      scene.push(Op::Style(Role::Reset));
    }
  }
}
