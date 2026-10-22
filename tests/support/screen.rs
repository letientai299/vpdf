use vpdf_model::geometry::Screen;

pub const fn pixels(
  columns: u16,
  rows: u16,
  cell_width: u16,
  cell_height: u16,
) -> Screen {
  Screen { columns, rows, cell_width, cell_height, origin: (0, 0) }
}

pub const fn cells(columns: u16, rows: u16) -> Screen {
  pixels(columns, rows, 8, 16)
}
