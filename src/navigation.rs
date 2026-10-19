//! Moving through the document: keys, marks, jumps, sections, scrolling.

use crate::Result;
use crate::app::{App, Request};
use crate::state::Mark;
use std::collections::BTreeMap;
use std::time::Instant;
use vpdf_model::document::Target;
use vpdf_model::text::{PageId, Position};
use vpdf_reader::reader::JumpDescription;
use vpdf_ui::keys::{Action, Input};
use vpdf_ui::panel::Row;
use vpdf_ui::reading::{ScrollInput, Sections, ViewportEvent};

impl App {
  pub fn navigate(&mut self, input: Input) -> Result<bool> {
    match input.action {
      Action::Section(forward) => self.move_section(forward, input.count)?,
      Action::History(forward) => {
        self.reader.cancel_scroll();
        for _ in 0..input.count {
          let Some(location) = self.reading_view().recall(forward)? else {
            break;
          };
          self.close_overview();
          self.reading_view().restore_history(location)?;
        }
      }
      Action::Quit => {
        self.quit = true;
        self.ensure_cursor()?;
      }
      _ => {
        let mut reading = self.reading_parts();
        return reading.view.navigate(input, reading.search);
      }
    }
    Ok(true)
  }

  pub fn viewport(&mut self, input: Input) -> Result<bool> {
    let mut reading = self.reading_parts();
    match reading.view.viewport(input, reading.search)? {
      ViewportEvent::Handled => {}
      ViewportEvent::Relayout => self.relayout()?,
      ViewportEvent::Crop => self.toggle_crop()?,
      ViewportEvent::Unhandled => return Ok(false),
    }
    Ok(true)
  }

  /// Zooms by `steps` of 15 %.
  fn zoom_by(&mut self, steps: f32) -> Result<()> {
    self.layout.request_zoom(vpdf_ui::layout::Zoom::Steps(steps));
    self.relayout()
  }

  /// Zooms keeping the page point at a page-area pixel in place, where
  /// the view allows.
  pub(super) fn zoom_at(&mut self, steps: f32, at: (f32, f32)) -> Result<()> {
    let anchor = self.anchor_at(at);
    self.zoom_by(steps)?;
    self.restore_at(anchor, at);
    Ok(())
  }

  pub(crate) fn pan(&mut self, delta: f32) -> Result<()> {
    self.reading_view().pan(delta)
  }
}

impl App {
  /// `m{a-z}` marks this document; `m{A-Z}` marks across documents.
  pub fn set_mark(&mut self, name: char) -> Result<()> {
    if name.is_ascii_uppercase() {
      self.request = Some(Request::SetMark(name));
    } else {
      let position = self.reading_position()?;
      self.marks.insert(name, position);
    }
    self.reader.notice(format!("Mark {name} set"));
    Ok(())
  }

  /// `'x` goes to the mark's line, `` `x `` to its exact glyph.
  pub fn go_mark(&mut self, name: char, exact: bool) -> Result<()> {
    if name.is_ascii_uppercase() {
      self.request = Some(Request::GoMark(name, exact));
      return Ok(());
    }
    let position = *self
      .marks
      .get(&name)
      .ok_or_else(|| anyhow::anyhow!("Mark not set: {name}"))?;
    self.go_to(position, exact)
  }

  /// Goes to a position: a shown one keeps the view, else it lands as
  /// followed links do.
  pub fn go_to(&mut self, position: Position, exact: bool) -> Result<()> {
    let target =
      if exact { position } else { Position { glyph: 0, ..position } };
    self.land_at(target)
  }

  pub fn go_to_page(&mut self, page: PageId) -> Result<()> {
    self.reading_view().go_to_page(page)
  }

  pub fn land_at(&mut self, position: Position) -> Result<()> {
    self.reading_view().land_at(position)
  }

  /// This document's marks, then global marks.
  pub fn mark_rows(&mut self, global: &BTreeMap<char, Mark>) -> Vec<Row> {
    let local: Vec<_> = self.marks.iter().map(|(n, p)| (*n, *p)).collect();
    let mut rows: Vec<_> = local
      .into_iter()
      .map(|(name, position)| {
        let text = self.doc.text(position.page).ok();
        Row::local_mark(name, position, text.as_deref())
      })
      .collect();
    rows.extend(
      global
        .iter()
        .map(|(name, mark)| Row::global_mark(*name, mark.cursor, &mark.path)),
    );
    rows
  }
}

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

impl App {
  pub fn animate(&mut self) -> bool {
    if !self.reader.animation().active() {
      return false;
    }
    self.reader.animate(Instant::now())
  }

  pub fn scroll(&mut self, delta: f32) -> Result<()> {
    self.search.forget_yank();
    self.reading_view().scroll(delta, ScrollInput::Wheel)
  }
}
