use vpdf_model::geometry::Screen;

pub fn cells(columns: u16, rows: u16) -> Screen {
  Screen { columns, rows, cell_width: 8, cell_height: 16, origin: (0, 0) }
}
