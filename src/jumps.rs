use crate::app::App;
use vpdf_model::document::Target;
use vpdf_model::text::Position;
use vpdf_reader::reader::JumpDescription;
use vpdf_ui::panel::Row;

impl App {
  pub(crate) fn link_description(
    &mut self,
    target: &Target,
    position: Position,
  ) -> JumpDescription {
    let text = self.doc.text(position.page).ok();
    vpdf_ui::panel::links::description(
      target,
      position,
      text.as_deref(),
      &self.doc.outline,
    )
  }

  pub(crate) fn jump_rows(&mut self) -> Vec<Row> {
    let jumps: Vec<_> = self
      .reader
      .described_jumps()
      .map(|(position, description)| (position, description.cloned()))
      .collect();
    jumps
      .into_iter()
      .map(|(position, description)| {
        let text = self.doc.text(position.page).ok();
        let line = text.as_ref().and_then(|text| text.lines.get(position.line));
        let name = self.doc.get_label(position.page).unwrap_or_default();
        Row::jump(position, description, line, name)
      })
      .collect()
  }
}
