use super::{
  App, Bounds, GlyphAt, Hit, List, MouseEventKind, PageId, Pixel, Position,
  Region, Result, Route,
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
      if down && self.text_break(mouse.row)? {
        return Ok(None);
      }
      return self.text_position(mouse.column, mouse.row).map(Some);
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
    Ok(self.doc.text(page)?.nearest(page, x, y))
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
    self.hit_at(Region::Page, column, row, pixel, Self::text_shown)
  }

  /// Page-area pixels at a cell: reported, else centre.
  pub(crate) fn cell_point(
    &self,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> (f32, f32) {
    // Clamped drags report pixels past the cell.
    let inside = |at: Option<u16>, cell: u32, size: u16| {
      at.map_or(f32::from(size) / 2.0, |at| {
        let offset = u32::from(at).saturating_sub(cell * u32::from(size));
        let last = size.saturating_sub(1);
        f32::from(u16::try_from(offset).unwrap_or(last).min(last))
      })
    };
    let (width, height) = (self.screen.cell_width, self.screen.cell_height);
    let (screen_column, screen_row) = (
      u32::from(self.at.column) + u32::from(column),
      u32::from(self.at.row) + u32::from(row),
    );
    let left = f32::from(column.saturating_sub(self.left)) * f32::from(width);
    let x = left + inside(pixel.map(|p| p.x), screen_column, width);
    let top = f32::from(row) * f32::from(height);
    (x, top + inside(pixel.map(|p| p.y), screen_row, height))
  }

  /// Page-area pixels of pane cells.
  pub(crate) fn cell_bounds(&self, cells: crate::split::Rect) -> Bounds {
    let screen = self.layout.screen;
    let (width, height) =
      (f32::from(screen.cell_width), f32::from(screen.cell_height));
    let x0 = f32::from(cells.column.saturating_sub(self.left)) * width;
    let y0 = f32::from(cells.row) * height;
    let x1 = x0 + f32::from(cells.columns) * width;
    Bounds { x0, y0, x1, y1: y0 + f32::from(cells.rows) * height }
  }

  /// Page and point under the pointer in a pane cell.
  pub fn page_point(
    &self,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> Option<(PageId, f32, f32)> {
    self.pixel_point(self.cell_point(column, row, pixel))
  }

  /// Page and point at a page-area pixel.
  pub(super) fn pixel_point(
    &self,
    (x, y): (f32, f32),
  ) -> Option<(PageId, f32, f32)> {
    let page = self.layout.page_at(y + self.reader.scroll);
    let (x, y) = self.layout.unproject(page, (x, y), self.reader.scroll)?;
    Some((page, x, y))
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
    self.hit_at(region, column, row, pixel, Self::text_hit)
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
        if let Some(index) = panel.row_at(column, row) {
          return Ok(Hit::Row(index));
        }
        let card = panel.shelf.as_ref().is_some_and(|s| s.covers(column));
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
        let follows = peek.is_some_and(crate::preview::Preview::follows);
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
