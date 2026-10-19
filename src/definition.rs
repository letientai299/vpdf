use crate::Result;
use crate::app::App;
use crate::document::Target;
use crate::reference::{Lookup, Reference};
use vpdf_search::query::Query;
use vpdf_ui::find::Phase;
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
    if let Some(target) = self.link_target(position)? {
      if matches!(target, Target::Uri(_)) {
        self.reader.notice("External link: use gx");
        return Ok(());
      }
      return self.follow(&target);
    }
    let text = self.doc.text(position.page)?;
    let Some(line) = text.lines.get(position.line) else { return Ok(()) };
    let byte =
      line.glyphs.iter().take(position.glyph).map(|g| g.ch.len_utf8()).sum();
    let Some(reference) = Reference::at(&line.text(), byte) else {
      self.reader.notice("No document reference at target");
      return Ok(());
    };
    if self.definition_heading(&reference)? {
      return Ok(());
    }
    let query = Query {
      text: reference.title(),
      reference: Some(Lookup { reference, origin: position }),
      ..Query::default()
    };
    self.start_query(&query)?;
    self.search.define(position);
    self.reader.notice("Finding reference…");
    Ok(())
  }

  fn definition_heading(&mut self, reference: &Reference) -> Result<bool> {
    let mut targets = Vec::new();
    for (index, heading) in self.doc.outline.iter().enumerate() {
      if !reference.heading(&heading.title) {
        continue;
      }
      let page = match heading.target {
        Target::Page(page) | Target::Point { page, .. } => page,
        Target::Uri(_) => continue,
      };
      if page.0 < self.doc.pages.len()
        && !targets.iter().any(|(_, target)| target == &heading.target)
      {
        targets.push((index, heading.target.clone()));
      }
    }
    match targets.as_slice() {
      [] => return Ok(false),
      [(_, target)] => self.follow(target)?,
      [(first, _), ..] => {
        self.open_panel(PanelKind::Outline)?;
        if let Some(panel) = &mut self.panel {
          panel.view.select(*first);
        }
        self
          .reader
          .notice("Multiple heading candidates; choose an outline entry");
      }
    }
    Ok(true)
  }

  pub(crate) fn finish_definition(&mut self) -> Result<()> {
    match self.search.hits() {
      [] => self.reader.notice("Reference not found"),
      [hit] => {
        let position = hit.first;
        self.go_to(position, true)?;
        self.reader.describe_jump(Some(self.search_description(position)));
        self.reader.notice("Reference found");
      }
      _ => {
        self.open_panel(PanelKind::Results)?;
        self.reader.notice("Multiple reference candidates; choose a result");
      }
    }
    Ok(())
  }
}
