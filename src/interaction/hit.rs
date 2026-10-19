use super::{
  App, GlyphAt, Hit, List, MouseEventKind, PageId, Pixel, Position, Region,
  Result, Route,
};

impl App {
  /// Shown glyph nearest a press.
  pub(crate) fn press_position(
    &mut self,
    route: &Route,
  ) -> Result<Option<Position>> {
    let mouse = route.mouse;
    if self.text_mode {
      // Presses on rules, like page gaps, miss.
      let down = matches!(mouse.kind, MouseEventKind::Down(_));
      if down && self.text().is_break(mouse.row)? {
        return Ok(None);
      }
      return self.text().position(mouse.column, mouse.row).map(Some);
    }
    let point = self.page_point(mouse.column, mouse.row, route.pixel);
    let Some((page, x, y)) = point else { return Ok(None) };
    self.glyph_near(page, x, y).map(Some)
  }

  /// The glyph nearest a page point.
  pub(crate) fn glyph_near(
    &mut self,
    page: PageId,
    x: f32,
    y: f32,
  ) -> Result<Position> {
    self.page_view().nearest_at(page, (x, y))
  }

  /// Link at a pressed, shown cell.
  pub(crate) fn link_pressed(
    &mut self,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> Result<Option<crate::document::Target>> {
    let hit = self.pressed_hit(column, row, pixel)?;
    self.link_of(hit)
  }

  /// The target of a link hit.
  pub(crate) fn link_of(
    &mut self,
    hit: Hit,
  ) -> Result<Option<crate::document::Target>> {
    let Hit::Link(page, index) = hit else { return Ok(None) };
    Ok(self.link(page, index)?.map(|link| link.target))
  }

  /// What a pressed, shown page cell hits.
  pub(crate) fn pressed_hit(
    &mut self,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> Result<Hit> {
    self.hit_at(Region::Page, column, row, pixel, |app, column, row| {
      app.text().shown(column, row)
    })
  }

  /// Page-area pixels at a cell: reported, else centre.
  pub(crate) fn cell_point(
    &self,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> (f32, f32) {
    vpdf_ui::paint::Frame {
      at: self.at,
      screen: self.screen,
      left: self.left,
      text: self.text_mode,
    }
    .point(column, row, pixel)
  }

  /// Page and point under the pointer in a pane cell.
  pub fn page_point(
    &self,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> Option<(PageId, f32, f32)> {
    self
      .layout
      .point_at(self.cell_point(column, row, pixel), self.reader.scroll)
  }

  /// A cell's point in `page` coordinates.
  pub(crate) fn point_on(
    &self,
    page: PageId,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> Option<(f32, f32)> {
    let at = self.cell_point(column, row, pixel);
    self.layout.unproject(page, at, self.reader.scroll)
  }

  /// What the pointer covers next frame.
  pub fn hovered(
    &mut self,
    region: Region,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> Result<Hit> {
    self.hit_at(region, column, row, pixel, |app, column, row| {
      app.text().hit(column, row)
    })
  }

  /// What a cell covers; `glyph` reads text.
  fn hit_at(
    &mut self,
    region: Region,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
    glyph: GlyphAt,
  ) -> Result<Hit> {
    match region {
      Region::Page if self.on_bar(column) => return Ok(Hit::Scrollbar),
      Region::Page if self.hints.is_some() => {
        if let Some(index) = self.items(List::Hints).at(column, row) {
          return Ok(Hit::Item(List::Hints, Some(index)));
        }
      }
      Region::Page => {}
      Region::Panel => {
        let Some(panel) = &self.panel else { return Ok(Hit::Nothing) };
        if let Some(index) = panel.view.row_at(column, row) {
          return Ok(Hit::Row(index));
        }
        let card = panel.view.shelf().is_some_and(|s| s.covers(column));
        return Ok(if card { Hit::Card(row) } else { Hit::Nothing });
      }
      Region::Overview => {
        let page = self.thumbnail_at(column, row);
        return Ok(page.map_or(Hit::Nothing, Hit::Thumbnail));
      }
      Region::Menu | Region::Actions => {
        let list =
          if region == Region::Menu { List::Menu } else { List::Actions };
        return Ok(Hit::Item(list, self.items(list).at(column, row)));
      }
      Region::Preview => {
        let peek = self.peek.as_ref();
        let follows = peek.is_some_and(vpdf_ui::preview::Preview::follows);
        return Ok(Hit::Preview { follows });
      }
      Region::Overlay => {
        let list = self.owned_list();
        let list =
          list.filter(|list| matches!(list, List::Clues | List::Completions));
        let index = list.and_then(|list| self.items(list).at(column, row));
        return Ok(list.map_or(Hit::Nothing, |list| Hit::Item(list, index)));
      }
      Region::Status => {
        let item = self.chrome_at(column, row);
        return Ok(item.map_or(Hit::Nothing, |_| Hit::Chrome));
      }
      Region::Edge => return Ok(Hit::Nothing),
    }
    if self.text_mode {
      let Some(position) = glyph(self, column, row)? else {
        return Ok(Hit::Nothing);
      };
      let point = self.glyph_point(position)?;
      let Some((page, x, y)) = point else { return Ok(Hit::Nothing) };
      return Ok(self.marked_hit(page, x, y)?.unwrap_or(Hit::Text));
    }
    let Some((page, x, y)) = self.page_point(column, row, pixel) else {
      return Ok(Hit::Nothing);
    };
    if let Some(hit) = self.marked_hit(page, x, y)? {
      return Ok(hit);
    }
    let text = self.doc.text(page)?;
    Ok(if text.covers(x, y) { Hit::Text } else { Hit::Nothing })
  }

  /// A link, else a note, here.
  fn marked_hit(
    &mut self,
    page: PageId,
    x: f32,
    y: f32,
  ) -> Result<Option<Hit>> {
    if let Some(index) = self.link_at(page, x, y)? {
      return Ok(Some(Hit::Link(page, index)));
    }
    let Some(index) = self.note_at(page, x, y)? else { return Ok(None) };
    let text = !self.note_icon(page, index);
    Ok(Some(Hit::Note { page, index, text }))
  }
}
