//! Overlays: input ownership, picked items, link hints, and key clues.

use crate::Result;
use crate::app::{App, Request};
use crossterm::event::{KeyEvent, MouseEvent};
use std::io::Write;
use std::time::{Duration, Instant};
use vpdf_model::document::Target;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::hints::Transition;
use vpdf_ui::input::{Dismiss, InputOwner, Targets};
use vpdf_ui::paint::Frame;
use vpdf_ui::pointer::{Hit, List};
use vpdf_ui::region::BAR_HINT;

impl App {
  pub(crate) fn pointer_targets(&self) -> Targets<'_> {
    let panel = self.panel.as_ref().map(|panel| &panel.view);
    self.overlays.targets(self.screen, panel)
  }

  /// Runs an item; its overlay may close.
  pub(crate) fn run_item(&mut self, list: List, index: usize) -> Result<()> {
    match list {
      List::Menu => {
        self.overlays.pick(list, index);
        self.context_run()
      }
      List::Actions => {
        self.overlays.pick(list, index);
        if let Some(operation) = vpdf_ui::region::Operation::at(index) {
          self.region_operation(operation)?;
        }
        Ok(())
      }
      List::Hints => self.run_hint(index),
      List::Clues => {
        let keys =
          self.overlays.clues().shown().and_then(|clues| clues.keys(index));
        // The session types them, as keys.
        self.request = keys.map(Request::Keys);
        Ok(())
      }
      List::Completions => {
        if let Some(prompt) = self.overlays.prompt_mut() {
          prompt.pick(index);
        }
        Ok(())
      }
    }
  }

  /// The `list` item under the pointer.
  pub(crate) fn pointed_item(&self, list: List) -> Option<usize> {
    self.hover.hit().item(list)
  }

  /// Motion picks the owning list's item.
  pub(crate) fn moved_onto(&mut self, hit: Hit) {
    if let Some((list, index)) = self.pointer_targets().hovered(hit) {
      self.overlays.pick(list, index);
    }
  }

  /// Clicks run the owning list's item.
  pub(crate) fn item_press(&mut self, mouse: MouseEvent) -> Result<bool> {
    let (column, row) = self.pane_cell(mouse);
    let mouse = MouseEvent { column, row, ..mouse };
    let Some((list, index)) = self.pointer_targets().pressed(mouse) else {
      return Ok(false);
    };
    self.run_item(list, index).map(|()| true)
  }
}

impl App {
  pub fn input_owner(&self) -> InputOwner {
    self.overlays.owner(self.panel_focused())
  }

  pub(crate) fn dismiss_overlay(&mut self) {
    self.end_gestures();
    // The bar's key hint goes with it.
    if self.overlays.dismiss() && self.reader.message == BAR_HINT {
      self.reader.notice("");
    }
  }

  /// Ends pending keys and notes.
  pub(crate) fn interrupt(&mut self) {
    self.disarm();
    self.clear_notes();
  }

  /// Drops the error and the message.
  pub(crate) fn clear_notes(&mut self) {
    self.error = None;
    self.reader.notice("");
  }

  /// Whether a press only dismisses something.
  pub(crate) fn overlaid(&self) -> bool {
    self.overlays.clues().shown().is_some() || self.input_owner().is_overlay()
  }

  pub(crate) fn dismiss_click(&mut self) -> Result<bool> {
    match self.input_owner().dismiss(self.overlays.clues().shown().is_some()) {
      Dismiss::None => return Ok(false),
      Dismiss::Prompt => {
        self.cancel_input()?;
        return Ok(true);
      }
      Dismiss::Hints => drop(self.overlays.close_hints()),
      Dismiss::Peek => self.dismiss_overlay(),
      Dismiss::Overlay => {
        self.dismiss_overlay();
        self.disarm();
        return Ok(true);
      }
      Dismiss::Reading => {}
    }
    self.reader.cancel();
    self.disarm();
    Ok(true)
  }
}

impl App {
  /// Labels links in view; `copy` copies URIs instead of opening them.
  pub fn start_hints(&mut self, copy: bool) -> Result<()> {
    let hints = self.reading_view().hints(copy)?;
    self.overlays.open_hints(hints);
    Ok(())
  }

  pub fn hint_key(&mut self, key: KeyEvent) -> Result<()> {
    let Some(hints) = self.overlays.hints_mut() else { return Ok(()) };
    match hints.key(key) {
      Transition::Stay => Ok(()),
      Transition::Run(index) => self.run_hint(index),
      Transition::Cancel => {
        self.overlays.close_hints();
        self.reader.cancel();
        self.disarm();
        Ok(())
      }
      Transition::Close => {
        self.overlays.close_hints();
        Ok(())
      }
      Transition::Miss(typed) => {
        self.overlays.close_hints();
        anyhow::bail!("No link labeled {typed}")
      }
    }
  }

  pub(crate) fn run_hint(&mut self, index: usize) -> Result<()> {
    let Some(hints) = self.overlays.close_hints() else { return Ok(()) };
    let Some((target, copy)) = hints.into_target(index) else { return Ok(()) };
    match target {
      Target::Uri(uri) if copy => self.copy_address(uri),
      target => self.follow(&target),
    }
  }

  pub fn draw_hints(
    &mut self,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<()> {
    let mut painter =
      vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
    self.scene_hints(&mut painter);
    painter.finish()?;
    Ok(())
  }

  pub(crate) fn scene_hints<'a>(
    &self,
    scene: &mut impl vpdf_model::scene::Draw<'a>,
  ) {
    let Some(hints) = self.overlays.hints() else { return };
    hints.scene(
      scene,
      Frame {
        at: self.at,
        screen: self.layout.screen(),
        left: self.left,
        text: self.text_mode,
      },
    );
  }
}

impl App {
  pub fn clue_due(&mut self) -> bool {
    self.clue_due_at(Instant::now())
  }

  pub(crate) fn clue_due_at(&mut self, now: Instant) -> bool {
    if self.overlays.prompt().is_some() || !self.overlays.clues().pending() {
      return false;
    }
    if !self.overlays.clues_mut().poll(now) {
      return false;
    }
    let registers = self
      .reader
      .registers
      .entries()
      .map(|(&name, text)| (name, text.as_ref()));
    let marks = self.marks.iter().map(|(&name, &at)| (name, at));
    let owner = self.input_owner();
    let grammar =
      owner.grammar(self.reader.mode(), self.reader.operator.is_some());
    let items = vpdf_ui::clues::items(
      &self.keys,
      grammar,
      |action| {
        let panel = self.panel.as_ref().map(|panel| &panel.view);
        self.overlays.allows(owner, action, panel)
      },
      registers,
      marks,
    );
    self.overlays.clues_mut().show(items)
  }

  pub fn clue_wait(&self) -> Option<Duration> {
    self.clue_wait_at(Instant::now())
  }

  pub(crate) fn clue_wait_at(&self, now: Instant) -> Option<Duration> {
    if self.overlays.prompt().is_some() || !self.overlays.clues().pending() {
      return None;
    }
    self.overlays.clues().wait(now)
  }

  pub fn after_key(&mut self) {
    self
      .overlays
      .clues_mut()
      .restart(self.keys.awaiting().map(|_| Instant::now()));
  }

  pub(crate) fn scroll_clues(&mut self, rows: isize) {
    self.overlays.clues_mut().scroll(rows, self.screen);
  }

  pub fn clue_rows(&self) -> u16 {
    self.overlays.clues().rows(self.screen)
  }
}
