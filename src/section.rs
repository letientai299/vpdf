use crate::Result;
use crate::app::App;
use vpdf_ui::reading::Sections;

impl App {
  pub(crate) fn move_section(
    &mut self,
    forward: bool,
    count: usize,
  ) -> Result<()> {
    self.want_outline();
    self.outline_updates()?;
    let sections = Sections::new(
      self.doc.outline.iter().map(|heading| &heading.target),
      self.doc.pages.len(),
    );
    let Some(section) =
      self.reading_view().section(sections, forward, count)?
    else {
      return Ok(());
    };
    self.follow(&section.target())?;
    self.reading_view().land_section(&section)
  }
}
