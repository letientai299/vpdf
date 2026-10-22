use anyhow::Result;
use std::hint::black_box;
use vpdf_model::document::Target;
use vpdf_ui::reading::Sections;
use vpdf_ui::text_view::TextView;

#[derive(serde::Deserialize)]
struct Prepared {
  headings: Vec<(String, usize, Target)>,
}

pub fn select() -> Result<()> {
  let prepared = super::fixtures::text()?;
  let outline: Prepared = super::fixture("scrollbar")?;
  let targets: Vec<_> =
    outline.headings.into_iter().map(|(_, _, t)| t).collect();
  anyhow::ensure!(!targets.is_empty(), "Missing benchmark sections");
  for text in [false, true] {
    for cursor in [false, true] {
      case(&prepared, &targets, text, cursor)?;
    }
  }
  Ok(())
}

fn case(
  prepared: &super::fixtures::TextPages,
  targets: &[Target],
  text: bool,
  cursor: bool,
) -> Result<()> {
  let screen = crate::screen::cells(100, 30);
  let mut layout = super::reading::layout(prepared, screen);
  let mut pane = super::reading::pane(prepared, cursor)?;
  let mut text_view = TextView::default();
  let frame = super::reading::frame(screen, text);
  let mut view = pane.bind(&mut layout, &mut text_view, frame, prepared);
  let sections = || Sections::new(targets.iter(), prepared.metadata.len());
  let expected = view.section(sections(), true, 1)?;
  anyhow::ensure!(expected.is_some(), "Missing next section");
  super::measure(
    &format!("Reading::section text={text} cursor={cursor}"),
    || {
      let section = view
        .section(sections(), true, black_box(1))
        .expect("prepared headings");
      black_box(section.map(|section| section.target()));
    },
  )
}
