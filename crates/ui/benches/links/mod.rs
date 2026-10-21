use anyhow::Result;
use std::hint::black_box;
use std::time::Instant;
use vpdf_model::document::Target;
use vpdf_model::input::{MouseButton, MouseEventKind};
use vpdf_model::text::PageId;
use vpdf_ui::hints::Spot;
use vpdf_ui::input::links::{Hover, Press, Step};
use vpdf_ui::pointer::Hit;

#[derive(serde::Deserialize)]
struct Prepared {
  page: PageId,
  found: Vec<(Spot, Target)>,
}

pub fn transitions() -> Result<()> {
  let prepared: Prepared = super::fixture("hints")?;
  anyhow::ensure!(prepared.found.len() >= 2, "Missing prepared links");
  let mut hover = Hover::default();
  let now = Instant::now();
  let mut index = 0;
  super::measure("Hover::point", || {
    index = 1 - index;
    let link = (prepared.page, index);
    hover.point(Hit::Link(link.0, link.1), Some(link), || black_box(now));
    black_box(hover.hit());
  })?;
  let mut press = Press::default();
  let target = &prepared.found[0].1;
  super::measure("Press::follow", || {
    press.step(MouseEventKind::Down(MouseButton::Left), 1);
    press.link(Some(black_box(target).clone()));
    let step = press.step(MouseEventKind::Up(MouseButton::Left), 1);
    assert!(matches!(step, Step::Follow(_)));
    black_box(step);
  })
}
