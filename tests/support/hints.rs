use crate::fixtures::screen;
use anyhow::Result;
use vpdf_model::document::Target;
use vpdf_model::geometry::Screen;
use vpdf_model::text::PageId;
use vpdf_ui::hints::Hints;

pub fn labels() -> Result<(Hints, Screen)> {
  let hints =
    Hints::new(vec![(((4, 2), 5), Target::Page(PageId(0)))], false, 0, 20)?;
  let screen = screen::cells(20, 10);
  Ok((hints, screen))
}
