//! Moving through the document: keys, marks, jumps, sections, scrolling.

use crate::Result;
use crate::app::{App, Request};
use crate::state::Mark;
use std::collections::BTreeMap;
use std::time::Instant;
use vpdf_model::document::Target;
use vpdf_model::text::{PageId, Position};
use vpdf_reader::reader::JumpDescription;
use vpdf_ui::keys::{Action, Input, PanelKind};
use vpdf_ui::layout::Zoom;
use vpdf_ui::panel::{Destination, Row, jump_key};
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
      Action::Quit => self.quit = true,
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

  /// Zooms keeping the page point at a page-area pixel in place, where
  /// the view allows.
  pub(super) fn zoom_at(&mut self, zoom: Zoom, at: (f32, f32)) -> Result<()> {
    let anchor = self.anchor_at(at);
    self.layout.request_zoom(zoom);
    self.relayout()?;
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
      self.request = Some(Request::MarkSet);
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

  pub fn show_marks(&mut self, global: &BTreeMap<char, Mark>) -> Result<()> {
    let rows = self.mark_rows(global);
    self.show_panel(PanelKind::Marks, rows)
  }

  /// This document's marks, then global marks, each under a heading.
  pub fn mark_rows(&mut self, global: &BTreeMap<char, Mark>) -> Vec<Row> {
    let local: Vec<_> = self.marks.iter().map(|(n, p)| (*n, *p)).collect();
    let local = local
      .into_iter()
      .map(|(name, position)| {
        let text = self.doc.text(position.page).ok();
        let page = self.doc.label(position.page);
        Row::local_mark(name, position, text.as_deref(), page)
      })
      .collect();
    let global = global
      .iter()
      .map(|(name, mark)| Row::global_mark(*name, mark.cursor, &mark.path))
      .collect();
    Row::marks(local, global)
  }
}

impl App {
  pub(crate) fn link_description(
    &mut self,
    target: &Target,
    position: Position,
  ) -> JumpDescription {
    let text = self.doc.text(position.page).ok();
    vpdf_ui::citations::description(
      target,
      position,
      text.as_deref(),
      &self.doc.outline,
    )
  }

  /// Jump stops newest first, keyed by their distance from the cursor.
  pub(crate) fn jump_rows(&mut self) -> Vec<Row> {
    self.jump_rows_from(&[])
  }

  /// Jump rows, reusing `old` rows whose key and stop still match.
  fn jump_rows_from(&mut self, old: &[Row]) -> Vec<Row> {
    let stops: Vec<_> = self
      .reader
      .stops()
      .map(|(offset, position, description)| {
        (offset, position, description.cloned())
      })
      .collect();
    let now = stops.iter().position(|(offset, ..)| *offset == 0);
    let now_key = jump_key(0);
    let kept = old.len() == stops.len()
      && now == old.iter().position(|row| row.key == now_key);
    stops
      .into_iter()
      .enumerate()
      .map(|(index, (offset, position, description))| {
        if let Some(row) = old
          .get(index)
          .filter(|row| kept && offset != 0 && jumps_to(row, position))
        {
          return row.clone();
        }
        let text = self.doc.text(position.page).ok();
        let line = text.as_ref().and_then(|text| text.lines.get(position.line));
        let name = self.doc.get_label(position.page).unwrap_or_default();
        let row = Row::jump(position, description, line, name);
        Row { key: jump_key(offset), ..row }
      })
      .collect()
  }

  /// Refills an open Jumps panel when the stops move and marks `now`.
  pub fn track_jumps(&mut self) -> Result<()> {
    let Some(panel) = &self.panel else { return Ok(()) };
    if panel.view.kind() != PanelKind::Jumps {
      return Ok(());
    }
    let rows = panel.view.rows();
    let stops: Vec<_> =
      self.reader.stops().map(|(offset, at, _)| (offset, at)).collect();
    let now = stops.iter().position(|(offset, _)| *offset == 0);
    let now_key = jump_key(0);
    // Offsets run without gaps, so `now` places every key.
    let stale = rows.len() != stops.len()
      || now != rows.iter().position(|row| row.key == now_key)
      || rows.iter().zip(&stops).any(|(row, (_, at))| !jumps_to(row, *at));
    if stale {
      let old = rows.to_vec();
      let rows = self.jump_rows_from(&old);
      let note = self.panel_note(PanelKind::Jumps);
      if let Some(panel) = &mut self.panel {
        panel.view.refill(rows, note);
      }
    }
    if let Some(panel) = &mut self.panel {
      panel.view.mark_current(now);
    }
    Ok(())
  }

  /// Whether the selected panel row is the Jumps `now` row.
  pub(crate) fn selects_now(&self) -> bool {
    self.panel.as_ref().is_some_and(|panel| {
      panel.view.kind() == PanelKind::Jumps
        && panel
          .view
          .rows()
          .get(panel.view.selected())
          .is_some_and(|row| row.key == jump_key(0))
    })
  }

  /// Brings the cursor into view without recording a jump.
  pub(crate) fn land_here(&mut self) -> Result<()> {
    self.reading_view().land_here()
  }
}

fn jumps_to(row: &Row, at: Position) -> bool {
  matches!(row.destination,
    Destination::Jump { position, .. } if position == at)
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
    self.animate_at(Instant::now())
  }

  pub(crate) fn animate_at(&mut self, now: Instant) -> bool {
    if !self.reader.animation().active() {
      return false;
    }
    self.reader.animate(now)
  }

  pub fn scroll(&mut self, delta: f32) -> Result<()> {
    self.scroll_as(delta, ScrollInput::Wheel)
  }

  pub(crate) fn scroll_as(
    &mut self,
    delta: f32,
    input: ScrollInput,
  ) -> Result<()> {
    self.search.forget_yank();
    self.reading_view().scroll(delta, input)
  }
}
