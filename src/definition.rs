use crate::Result;
use crate::app::App;
use crate::document::Target;
use crate::find::{Phase, Query};
use crate::keys::PanelKind;
use crate::reference::{Lookup, Reference};

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
        self.reader.message = "External link: use gx".into();
        return Ok(());
      }
      return self.follow(&target);
    }
    let text = self.doc.text(position.page)?;
    let Some(line) = text.lines.get(position.line) else { return Ok(()) };
    let byte =
      line.glyphs.iter().take(position.glyph).map(|g| g.ch.len_utf8()).sum();
    let Some(reference) = Reference::at(&line.text(), byte) else {
      self.reader.message = "No document reference at target".into();
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
    self.reader.message = "Finding reference…".into();
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
          panel.select(*first);
        }
        self.reader.message =
          "Multiple heading candidates; choose an outline entry".into();
      }
    }
    Ok(true)
  }

  pub(crate) fn finish_definition(&mut self) -> Result<()> {
    match self.search.hits() {
      [] => self.reader.message = "Reference not found".into(),
      [hit] => {
        let position = hit.first;
        self.go_to(position, true)?;
        self.reader.describe_jump(Some(self.search_description(position)));
        self.reader.message = "Reference found".into();
      }
      _ => {
        self.open_panel(PanelKind::Results)?;
        self.reader.message =
          "Multiple reference candidates; choose a result".into();
      }
    }
    Ok(())
  }
}
