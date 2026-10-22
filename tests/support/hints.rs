use anyhow::Result;
use vpdf_model::document::Target;
use vpdf_model::geometry::Screen;
use vpdf_model::text::PageId;
use vpdf_ui::hints::Hints;

pub fn labels() -> Result<(Hints, Screen)> {
  let hints =
    Hints::new(vec![(((4, 2), 5), Target::Page(PageId(0)))], false, 0, 20)?;
  let screen = Screen {
    columns: 20,
    rows: 10,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  Ok((hints, screen))
}
