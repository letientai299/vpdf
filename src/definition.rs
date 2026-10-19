use crate::Result;
use crate::app::App;
use crate::pdf_text::PdfText;
use vpdf_ui::find::{Definition, Phase};
use vpdf_ui::keys::PanelKind;

impl App {
  pub(crate) fn cancel_definition(&mut self) {
    if matches!(self.search.phase(), Phase::Definition) {
      self.cancel_search();
    }
  }

  pub(crate) fn definition(&mut self) -> Result<()> {
    let Some(position) = self.explicit_target()? else { return Ok(()) };
    self.search.stop_jumping();
    let link = self.link_target(position)?;
    let definition =
      Definition::resolve(position, link, &mut PdfText(&mut self.doc))?;
    self.apply_definition(definition)
  }

  pub(crate) fn finish_definition(&mut self) -> Result<()> {
    self.apply_definition(self.search.definition_result())
  }

  fn apply_definition(&mut self, definition: Definition) -> Result<()> {
    let message = definition.message();
    match definition {
      Definition::Follow(target) => self.follow(&target)?,
      Definition::Outline(selected) => {
        self.open_panel(PanelKind::Outline)?;
        if let Some(panel) = &mut self.panel {
          panel.view.select(selected);
        }
      }
      Definition::Search(lookup) => {
        let origin = lookup.origin;
        self.start_query(&lookup.into())?;
        self.search.define(origin);
      }
      Definition::Found { position, description } => {
        self.go_to(position, true)?;
        self.reader.describe_jump(Some(description));
      }
      Definition::Candidates => self.open_panel(PanelKind::Results)?,
      Definition::Unchanged
      | Definition::External
      | Definition::NoReference
      | Definition::NotFound => {}
    }
    if let Some(message) = message {
      self.reader.notice(message);
    }
    Ok(())
  }
}
