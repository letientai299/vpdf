use super::Pane;
use super::{App, Result, Screen, Session};
use crate::manual::{Location, Runtime};
use vpdf_ui::topics::{Destination, Invocation, Lookup, Reference, TopicId};

#[derive(Default)]
pub(super) struct Help {
  runtime: Runtime,
  reference: Option<Reference>,
  cached: Option<(Reference, std::sync::Arc<vpdf_pdf::manual::Document>)>,
  pending: Option<Delivery>,
  return_to: Option<u32>,
  /// A refresh skipped help kept only as an alternate.
  stale: bool,
}

#[derive(Clone, Copy)]
enum Delivery {
  Open(Option<u32>, bool),
  Refresh,
}

impl App {
  pub(crate) fn document_identity(&self) -> &std::path::Path {
    if self.manual.is_some() {
      std::path::Path::new(crate::manual::ID)
    } else {
      &self.path
    }
  }

  fn help_location(&mut self) -> Option<Location> {
    let cursor = self.reader.cursor;
    if self.reader.viewport_mode()
      && !self.text_mode
      && self.reading_view().matches_landing(cursor)
      && let Some(vpdf_model::document::Target::Point { page, y, .. }) =
        self.reader.landing().aimed()
    {
      return Location::at(self.manual.as_ref()?, *page, *y);
    }
    if self.reader.viewport_mode() && !self.text_mode {
      let (page, (_, y)) =
        self.anchor_at((self.layout.screen().width() / 2.0, 0.0));
      return Location::at(self.manual.as_ref()?, page, y);
    }
    let position = if self.text_mode && self.reader.viewport_mode() {
      self.text_view.top()
    } else {
      self.navigation_position()
    };
    match self.glyph_point(position).ok().flatten() {
      Some((_, x, y)) => {
        Location::point(self.manual.as_ref()?, position.page, x, y)
      }
      None
        if self.text_mode
          && self.doc.text(position.page).is_ok_and(|text| {
            !text.lines.is_empty() && position.line >= text.lines.len()
          }) =>
      {
        Location::page_break(self.manual.as_ref()?, position.page)
      }
      None => Location::at(self.manual.as_ref()?, position.page, 0.0),
    }
  }

  fn restore_help_position(
    &mut self,
    target: &vpdf_model::document::Target,
    viewport: bool,
    location: Option<&Location>,
  ) -> Result<()> {
    self.follow(target)?;
    if !viewport {
      self.reader.enter_cursor();
    } else if self.text_mode {
      let mut position = self.reader.cursor;
      if location.is_some_and(Location::is_page_break) {
        position.line = self.doc.text(position.page)?.lines.len();
        position.glyph = 0;
      }
      let frame =
        vpdf_ui::text_view::Frame::new(self.screen, self.at, self.left);
      self
        .text_view
        .bind(crate::pdf_text::PdfText(&mut self.doc), frame)
        .origin(position)?;
    }
    Ok(())
  }

  fn refresh_help_document(
    &mut self,
    ready: &crate::manual::Ready,
    settings: &crate::config::Settings,
  ) -> Result<()> {
    let location = self.help_location();
    let mut replacement = Self::help_document(
      ready.document.clone(),
      self.screen,
      self.frontend.clone(),
      settings,
    )?;
    replacement.id = self.id;
    replacement.document_generation = self.document_generation + 1;
    replacement.images_stale = true;
    replacement.layout.copy_view(&self.layout);
    replacement.layout.clear_crops();
    replacement
      .layout
      .reflow(replacement.doc.pages.iter().map(|page| page.bounds));
    replacement.text_mode = self.text_mode;
    replacement.preview = self.preview;
    if !replacement.layout.crop().is_none() {
      replacement.start_crop();
    }
    let target = location
      .as_ref()
      .and_then(|location| location.target(&ready.document))
      .unwrap_or_else(|| ready.target.clone());
    replacement.restore_help_position(
      &target,
      self.reader.viewport_mode(),
      location.as_ref(),
    )?;
    *self = replacement;
    Ok(())
  }
}

impl Session {
  fn help_reference(&self, invocation: Invocation) -> Reference {
    self.settings.help_reference(invocation)
  }

  pub(super) fn find_pane(&self, id: u32) -> Option<(usize, usize)> {
    self
      .tabs
      .iter()
      .enumerate()
      .find_map(|(tab, panes)| panes.index(id).map(|pane| (tab, pane)))
  }

  pub(super) fn request_help(
    &mut self,
    query: &str,
    invocation: Invocation,
    screen: Screen,
  ) -> Result<()> {
    self.request_help_at(query, invocation, screen, false)
  }

  pub(super) fn request_help_at(
    &mut self,
    query: &str,
    invocation: Invocation,
    screen: Screen,
    beside: bool,
  ) -> Result<()> {
    if self.tabs.is_empty() {
      self.recent_tab(screen)?;
    }
    let full_query = query;
    let explicit = query.split_once(':').is_some_and(|(name, _)| {
      vpdf_ui::bindings::Context::parse(name).is_some()
    });
    let (context, query) =
      vpdf_ui::topics::query_context(query, invocation.context());
    let mut reference = self.help_reference(invocation.in_context(context));
    let registry = crate::manual::registry(&reference)?;
    let query = if query.is_empty() { "introduction" } else { query };
    let location = match registry.lookup(query, reference.invocation.context())
    {
      Lookup::Found(
        Destination::Authored(id)
        | Destination::Action(id)
        | Destination::Command(id)
        | Destination::Catalog(id),
      ) => Location::topic(id),
      Lookup::Found(Destination::Key(key)) => {
        Location::anchored(key.action.clone(), &key.anchor())?
      }
      Lookup::Found(Destination::Configuration) => {
        Location::topic(TopicId::parse("config")?)
      }
      Lookup::Found(Destination::Setting(id)) => {
        Location::anchored(TopicId::parse("config")?, id.as_str())?
      }
      Lookup::Missing => {
        let suggestions = registry.suggest(query, context);
        if suggestions.is_empty() {
          anyhow::bail!("No help: {full_query:?}; use :help then Tab");
        }
        anyhow::bail!(
          "No help: {full_query:?}; try :help {}",
          suggestions.join(", ")
        );
      }
      Lookup::Ambiguous(mut candidates) => {
        if explicit {
          for candidate in &mut candidates {
            candidate.name = format!("{}:{}", context.name(), candidate.name);
          }
        }
        let mut prompt =
          vpdf_ui::prompt::Prompt::new(vpdf_ui::prompt::PromptKind::Command)
            .with_invocation(reference.invocation);
        let command = if beside { "help!" } else { "help" };
        prompt.replace(&format!("{command} {full_query}"));
        prompt.choices(
          command.len() + 1,
          candidates.iter().map(|candidate| candidate.name.clone()).collect(),
        );
        prompt.describe(|name| crate::cmdline::describe(name, &candidates));
        if !self.tabs.is_empty() {
          self.focused_mut().open_prompt(prompt);
        }
        return Ok(());
      }
    };
    let source = self.tabs.get(self.active).map(|tab| &tab.panes[tab.focus]);
    let id = source.map(|app| app.id);
    if source.and_then(Pane::document).is_some_and(|app| app.manual.is_some())
      && let Some((previous, _)) = self.help.cached.clone()
    {
      // Keep the manual's source snapshot stable.
      if previous.configuration == reference.configuration && !explicit {
        self.restore_help();
        reference = previous;
      } else {
        reference = self.help_reference(if explicit {
          previous.invocation.in_context(context)
        } else {
          previous.invocation
        });
      }
    }
    self.screen.get_or_insert(screen);
    self.help.pending = Some(Delivery::Open(id, beside));
    self.help.runtime.request(&reference, location, &self.frontend.wake());
    self.help.reference = Some(reference);
    Ok(())
  }

  fn restore_help(&mut self) {
    if let Some((reference, document)) = self.help.cached.clone() {
      self.help.runtime.reuse(document);
      self.help.reference = Some(reference);
    }
  }

  pub(super) fn refresh_help(&mut self) {
    let Some(mut reference) = self.help.reference.clone() else { return };
    if self.help.pending.is_none() {
      let location = self
        .panes_mut()
        .find(|app| app.manual.is_some())
        .and_then(App::help_location);
      let Some(location) = location else {
        self.help.stale = true;
        return;
      };
      self.help.runtime.locate(location);
      self.help.pending = Some(Delivery::Refresh);
    }
    self.help.stale = false;
    reference = self.help_reference(reference.invocation);
    self.help.runtime.refresh(&reference, &self.frontend.wake());
    self.help.reference = Some(reference);
  }

  pub(super) fn help_updates(&mut self, out: &mut impl std::io::Write) -> bool {
    let Some(result) = self.help.runtime.poll() else { return false };
    self.help_result(result, out);
    true
  }

  fn help_result(
    &mut self,
    result: Result<crate::manual::Ready>,
    out: &mut impl std::io::Write,
  ) {
    let Some(delivery) = self.help.pending.take() else { return };
    let result =
      result.and_then(|ready| self.deliver_help(ready, delivery, out));
    if let Err(error) = result {
      self.restore_help();
      if !self.tabs.is_empty() {
        self.focused_mut().fail(error.to_string());
      }
    }
  }

  fn deliver_help(
    &mut self,
    ready: crate::manual::Ready,
    delivery: Delivery,
    _out: &mut impl std::io::Write,
  ) -> Result<()> {
    if let Delivery::Open(source, _) = delivery {
      let current =
        self.tabs.get(self.active).map(|tab| tab.panes[tab.focus].id);
      if current != source {
        self.restore_help();
        return Ok(());
      }
    }
    let screen = self
      .screen
      .ok_or_else(|| anyhow::anyhow!("Help needs screen dimensions"))?;
    let beside = matches!(delivery, Delivery::Open(Some(_), true));
    let existing = self.find_help(delivery);
    if matches!(delivery, Delivery::Refresh) && existing.is_none() {
      return Ok(());
    }
    if beside && existing.is_none() {
      self.check_split(super::Axis::Columns)?;
    }
    let cached = self
      .help
      .reference
      .clone()
      .map(|reference| (reference, ready.document.clone()));
    // Replace page-based state after reflow.
    for tab in &mut self.tabs {
      for app in tab.panes.iter_mut().filter_map(Pane::document_mut) {
        let Some(old) = &app.manual else { continue };
        if std::sync::Arc::ptr_eq(old, &ready.document) {
          continue;
        }
        app.refresh_help_document(&ready, &self.settings)?;
        self.graphics.forget(app.id);
      }
    }
    if matches!(delivery, Delivery::Refresh) {
      self.help.cached = cached;
      return self.arrange();
    }
    if let Some((tab, pane)) = existing {
      let app = self.tabs[tab].panes[pane]
        .document_mut()
        .ok_or_else(|| anyhow::anyhow!("No document"))?;
      app.close_panel()?;
      app.close_overview();
      app.follow(&ready.target)?;
      self.focus(tab, pane)?;
    } else {
      let mut app = App::help_document(
        ready.document,
        screen,
        self.frontend.clone(),
        &self.settings,
      )?;
      app.follow(&ready.target)?;
      self.leave()?;
      self.place_help(app, beside)?;
    }
    if let Delivery::Open(source, true) = delivery
      && source.and_then(|id| self.find_pane(id)).is_none_or(|(tab, pane)| {
        self.tabs[tab].panes[pane]
          .document()
          .is_none_or(|app| app.manual.is_none())
      })
    {
      self.help.return_to = source;
    }
    self.help.cached = cached;
    self.arrange()
  }

  fn find_help(&self, delivery: Delivery) -> Option<(usize, usize)> {
    let invoking_help = match delivery {
      Delivery::Open(Some(id), _) => {
        self.find_pane(id).filter(|&(tab, pane)| {
          self.tabs[tab].panes[pane]
            .document()
            .is_some_and(|app| app.manual.is_some())
        })
      }
      _ => None,
    };
    let local_help = self.tabs.get(self.active).and_then(|tab| {
      tab.panes.iter().position(Pane::is_help).map(|pane| (self.active, pane))
    });
    invoking_help.or(local_help).or_else(|| {
      self
        .tabs
        .iter()
        .enumerate()
        .filter(|_| matches!(delivery, Delivery::Refresh))
        .find_map(|(t, tab)| {
          tab.panes.iter().position(Pane::is_help).map(|p| (t, p))
        })
    })
  }

  /// Plain help replaces the focused pane, keeping it as alternate.
  fn place_help(&mut self, app: App, beside: bool) -> Result<()> {
    if beside {
      return self.insert_pane(
        app.into(),
        super::Axis::Columns,
        super::Place::After,
      );
    }
    self.replace_keeping_alternate(app.into())
  }

  /// Revives help held as an alternate after a skipped refresh.
  pub(super) fn revive_help(&mut self) {
    if self.focused().is_help() && std::mem::take(&mut self.help.stale) {
      self.refresh_help();
    }
  }

  pub(super) fn retarget_help(&mut self, old: u32, new: u32) {
    if self.help.return_to == Some(old) {
      self.help.return_to = Some(new);
    }
  }

  pub(super) fn help_invoker(&self) -> Option<(usize, usize)> {
    self.help.return_to.and_then(|id| self.find_pane(id))
  }

  pub(super) fn return_from_help(&mut self) -> Result<()> {
    if let Some((tab, pane)) = self.help_invoker() {
      return self.focus(tab, pane);
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests;
