//! Command and search line with zsh emacs-keymap editing.
//!
//! Beyond the emacs defaults, it mirrors zsh-autosuggestions (ghost text
//! from history, accepted with Ctrl-Space, Right, or End), Ctrl-w and
//! Ctrl-arrows that stop at punctuation, history search on Ctrl-r, and
//! `edit-command-line` on Ctrl-x Ctrl-e.

use crate::bindings::{
  Bindings, Context, Map, Match, Pending, PromptAction, Target,
};
use crate::grid::{self, Grid};
use crate::layout::Screen;
use crate::line::{self, NARROW, SHELL};
use std::sync::Arc;
use unicode_width::UnicodeWidthStr;
use vpdf_model::geometry::At;
use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};
use vpdf_model::scene::{Draw, Op, Text};
use vpdf_model::style::Role;

const LIMIT: usize = 4096;
const HISTORY: usize = 200;
const UNDO: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptKind {
  Search(bool),
  Command,
  Password,
}

#[derive(Clone, Default)]
pub struct History {
  commands: Vec<String>,
  searches: Vec<String>,
  /// Counts pushes, so a saver can tell when entries changed.
  revision: u64,
}

impl History {
  pub fn new(commands: Vec<String>, searches: Vec<String>) -> Self {
    Self { commands, searches, revision: 0 }
  }

  pub fn commands(&self) -> &[String] {
    &self.commands
  }
  pub fn searches(&self) -> &[String] {
    &self.searches
  }
  pub fn revision(&self) -> u64 {
    self.revision
  }

  fn entries(&self, kind: PromptKind) -> &[String] {
    match kind {
      PromptKind::Search(_) => &self.searches,
      PromptKind::Command => &self.commands,
      PromptKind::Password => &[],
    }
  }

  pub fn push(&mut self, kind: PromptKind, text: &str) {
    let entries = match kind {
      PromptKind::Search(_) => &mut self.searches,
      PromptKind::Command => &mut self.commands,
      PromptKind::Password => return,
    };
    if text.trim().is_empty() {
      return;
    }
    entries.retain(|entry| entry != text);
    entries.push(text.to_owned());
    if entries.len() > HISTORY {
      entries.remove(0);
    }
    self.revision += 1;
  }
}

/// Completion candidates replacing `original[start..end]`, listed like
/// zsh; `selected` is set once Tab starts cycling.
pub struct Menu {
  items: Vec<String>,
  /// What the list shows: the last component of each path.
  labels: Vec<String>,
  selected: Option<usize>,
  start: usize,
  end: usize,
  original: String,
  guidance: bool,
}

impl Menu {
  pub fn items(&self) -> &[String] {
    &self.items
  }
  pub fn labels(&self) -> &[String] {
    &self.labels
  }
  pub fn selected(&self) -> Option<usize> {
    self.selected
  }

  pub fn layout(&self, columns: usize, limit: usize) -> Grid {
    grid::layout(&self.labels, columns, limit, self.selected)
  }

  /// Layout above the status row.
  pub fn grid(&self, screen: Screen) -> Grid {
    self.layout(
      usize::from(screen.columns),
      grid::limit(screen)
        .min(usize::from(
          screen.rows.saturating_sub(1 + self.footer_rows(screen)),
        ))
        .max(1),
    )
  }

  pub fn footer_rows(&self, screen: Screen) -> u16 {
    u16::from(self.guidance && screen.rows >= 3)
  }
}

/// Position in history while browsing, and the line before browsing.
struct Recall {
  index: usize,
  saved: String,
  /// Ctrl-r search text; `None` for plain Up and Down.
  pattern: Option<String>,
}

pub struct Prompt {
  invocation: Option<crate::topics::Invocation>,
  kind: PromptKind,
  text: String,
  menu: Option<Menu>,
  /// History suffix offered after the cursor, like zsh-autosuggestions.
  ghost: String,
  cursor: usize,
  anchor: Option<usize>,
  native: bool,
  recall: Option<Recall>,
  kill: String,
  undo: Vec<(String, usize)>,
  bindings: Arc<Map<Target>>,
  command: Option<String>,
  pending: Pending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptEvent {
  Pending,
  Edited,
  Complete,
  /// Edit the line in `$EDITOR`.
  Editor,
  Submit,
  Cancel,
  Command,
}

impl Prompt {
  pub fn scene<'a>(
    &self,
    scene: &mut impl Draw<'a>,
    screen: Screen,
    at: At,
    pointed: Option<usize>,
  ) {
    if screen.rows == 0 || screen.columns == 0 {
      return;
    }
    let row = screen.rows - 1;
    if let Some(menu) = &self.menu
      && row > 0
    {
      let layout = menu.grid(screen);
      let footer = menu.footer_rows(screen);
      let bottom = (at, row - 1 - footer);
      let shown = (menu.selected, pointed);
      grid::scene(scene, bottom, &menu.labels, &layout, shown, 0, screen);
      if footer > 0 {
        crate::paint::move_to(scene, at, row - 1, 0u16);
        scene.push(Op::Style(Role::Bar));
        let text = self.completion_hint(menu.selected.is_some());
        let text = vpdf_model::text::clean(&text, usize::from(screen.columns));
        let pad = usize::from(screen.columns).saturating_sub(text.width());
        scene.extend([Op::Text(Text::new(text)), Op::Spaces(pad)]);
      }
    }
    let (text, ghost, column) = self.display();
    let room = usize::from(screen.columns).saturating_sub(text.width());
    crate::paint::move_to(scene, at, row, 0u16);
    scene.push(Op::Text(Text::new(vpdf_model::text::clean(
      &text,
      usize::from(screen.columns),
    ))));
    let ghost = vpdf_model::text::clean(ghost, room);
    let pad = room.saturating_sub(ghost.width());
    scene.extend([
      Op::Style(Role::Muted),
      Op::Text(Text::new(ghost)),
      Op::Style(Role::Reset),
      Op::Spaces(pad),
    ]);
    if let Some(range) = self.selection() {
      let (start, selected) = if self.kind == PromptKind::Password {
        (
          10 + self.text[..range.start].chars().count(),
          "*".repeat(self.text[range].chars().count()),
        )
      } else {
        (1 + self.text[..range.start].width(), self.text[range].to_owned())
      };
      let room = usize::from(screen.columns).saturating_sub(start);
      if room > 0 {
        crate::paint::move_to(scene, at, row, start);
        scene.push(Op::Style(Role::TextSelection));
        scene
          .push(Op::Text(Text::new(vpdf_model::text::clean(&selected, room))));
        scene.push(Op::Style(Role::Reset));
      }
    }
    let column = column.min(usize::from(screen.columns) - 1);
    crate::paint::move_to(scene, at, row, column);
    scene.extend([
      Op::InputArea {
        masked: self.kind == PromptKind::Password,
        column: usize::from(at.column),
        columns: usize::from(screen.columns),
      },
      Op::BlockCursor,
      Op::ShowCursor,
    ]);
  }

  pub fn kind(&self) -> PromptKind {
    self.kind
  }
  pub fn text(&self) -> &str {
    &self.text
  }
  pub fn into_text(self) -> String {
    self.text
  }
  pub fn menu(&self) -> Option<&Menu> {
    self.menu.as_ref()
  }

  pub fn new(kind: PromptKind) -> Self {
    Self {
      invocation: None,
      kind,
      text: String::new(),
      menu: None,
      ghost: String::new(),
      cursor: 0,
      anchor: None,
      native: false,
      recall: None,
      kill: String::new(),
      undo: Vec::new(),
      bindings: Bindings::default().map(Context::prompt(kind)),
      command: None,
      pending: Pending::default(),
    }
  }

  #[must_use]
  pub fn with_invocation(
    mut self,
    invocation: crate::topics::Invocation,
  ) -> Self {
    self.set_invocation(invocation);
    self
  }

  pub fn set_invocation(&mut self, invocation: crate::topics::Invocation) {
    self.invocation = Some(invocation);
  }

  pub fn invocation(&self) -> Option<&crate::topics::Invocation> {
    self.invocation.as_ref()
  }

  pub fn before_cursor(&self) -> &str {
    &self.text[..self.cursor]
  }

  /// Text, ghost suggestion, and cursor column as drawn.
  pub fn display(&self) -> (String, &str, usize) {
    let prefix = match self.kind {
      PromptKind::Search(false) => "/",
      PromptKind::Search(true) => "?",
      PromptKind::Command => ":",
      PromptKind::Password => {
        let stars = "*".repeat(self.text.chars().count());
        let column = self.before_cursor().chars().count();
        return (format!("Password: {stars}"), "", 10 + column);
      }
    };
    let column = prefix.width() + self.before_cursor().width();
    (format!("{prefix}{}", self.text), &self.ghost, column)
  }

  /// Replaces the whole line, as after editing it externally.
  pub fn replace(&mut self, text: &str) {
    self.snapshot();
    self.ghost.clear();
    self.text.clear();
    self.cursor = 0;
    self.anchor = None;
    self.insert(&text.replace(['\n', '\r'], " "));
  }

  /// Inserts pasted text as one undoable edit.
  pub fn paste(&mut self, text: &str) -> PromptEvent {
    if text.chars().all(char::is_control) {
      return PromptEvent::Pending;
    }
    self.snapshot();
    self.menu = None;
    self.recall = None;
    self.ghost.clear();
    let before = self.text.clone();
    self.insert(text);
    if self.text == before { PromptEvent::Pending } else { PromptEvent::Edited }
  }

  pub fn insert(&mut self, text: &str) {
    self.delete_selection();
    for ch in text.chars().filter(|ch| !ch.is_control()) {
      if self.text.len() + ch.len_utf8() > LIMIT {
        break;
      }
      self.text.insert(self.cursor, ch);
      self.cursor += ch.len_utf8();
    }
  }

  fn snapshot(&mut self) {
    if self.undo.last().is_none_or(|(text, _)| *text != self.text) {
      self.undo.push((self.text.clone(), self.cursor));
      if self.undo.len() > UNDO {
        self.undo.remove(0);
      }
    }
  }

  pub fn reset_input(&mut self) {
    self.pending.clear();
  }

  pub fn feed(&mut self, key: KeyEvent, history: &History) -> PromptEvent {
    let event = self.dispatch(key, history);
    self.suggest(history);
    event
  }

  pub fn set_bindings(&mut self, bindings: Arc<Map<Target>>) {
    if !Arc::ptr_eq(&self.bindings, &bindings) {
      self.bindings = bindings;
      self.reset_input();
    }
  }

  fn dispatch(&mut self, key: KeyEvent, history: &History) -> PromptEvent {
    if self.native
      && let Some(event) = self.native_key(key, history)
    {
      return event;
    }
    match self.pending.feed_cancellable(&self.bindings, key, Target::cancels) {
      Match::Target(Target::Prompt(action)) => self.action(action, history),
      Match::Target(Target::Command(command)) => {
        self.command = Some(command);
        PromptEvent::Command
      }
      Match::Unbound => {
        if let KeyCode::Char(ch) = key.code
          && crate::keys::plain(key)
        {
          self.menu = None;
          self.recall = None;
          self.snapshot();
          let ch = if key.modifiers.contains(KeyModifiers::SHIFT) {
            ch.to_ascii_uppercase()
          } else {
            ch
          };
          self.insert(ch.encode_utf8(&mut [0; 4]));
          return PromptEvent::Edited;
        }
        PromptEvent::Pending
      }
      Match::Target(_) | Match::Pending | Match::Consumed => {
        PromptEvent::Pending
      }
    }
  }

  pub fn take_command(&mut self) -> Option<String> {
    self.command.take()
  }

  pub fn action(
    &mut self,
    action: PromptAction,
    history: &History,
  ) -> PromptEvent {
    use PromptAction as A;
    if let A::Complete { forward } = action {
      self.anchor = None;
      return self.cycle(forward);
    }
    self.menu = None;
    if !matches!(action, A::History { .. }) {
      self.recall = None;
    }
    if matches!(
      action,
      A::Start
        | A::End
        | A::Move { .. }
        | A::Word { .. }
        | A::History { .. }
        | A::Transpose
    ) {
      self.anchor = None;
    }
    let before = self.text.clone();
    match action {
      A::Cancel => return PromptEvent::Cancel,
      A::Submit => return PromptEvent::Submit,
      A::Editor if self.kind != PromptKind::Password => {
        return PromptEvent::Editor;
      }
      A::Editor => {}
      A::Start => self.cursor = 0,
      A::End | A::Move { forward: true } if self.accept() => {}
      A::End => self.cursor = self.text.len(),
      A::Move { forward } => self.step(if forward { 1 } else { -1 }),
      A::Accept => {
        self.accept();
      }
      A::Word { forward, shell } => {
        let chars = if shell { SHELL } else { NARROW };
        if forward {
          self.forward_word(chars);
        } else {
          self.cursor = line::word_start(&self.text, self.cursor, chars);
        }
      }
      A::Delete { backward: true } => self.edit(|p| {
        if !p.delete_selection() {
          p.delete_back();
        }
      }),
      A::Delete { backward: false } | A::DeleteOrComplete => {
        if action == A::DeleteOrComplete && self.cursor == self.text.len() {
          return self.cycle(true);
        }
        self.edit(|p| {
          if !p.delete_selection() {
            p.remove(p.cursor..p.next_char());
          }
        });
      }
      A::Kill { to_start } => self.edit(|p| {
        if p.delete_selection() {
          return;
        }
        if to_start {
          p.remove(0..p.cursor);
        } else {
          p.remove(p.cursor..p.text.len());
        }
      }),
      A::DeleteWord { forward, shell } => self.edit(|p| {
        if p.delete_selection() {
          return;
        }
        let chars = if shell { SHELL } else { NARROW };
        if forward {
          p.remove(p.cursor..line::word_end(&p.text, p.cursor, chars));
        } else {
          p.remove(line::word_start(&p.text, p.cursor, chars)..p.cursor);
        }
      }),
      A::Yank => {
        let kill = self.kill.clone();
        self.edit(|p| p.insert(&kill));
      }
      A::Transpose => self.edit(Self::transpose),
      A::History { forward, search } => {
        self.recall(history, if forward { 1 } else { -1 }, search);
      }
      A::Undo => return self.restore(),
      A::Complete { .. } => unreachable!(),
    }
    if self.text == before { PromptEvent::Pending } else { PromptEvent::Edited }
  }

  pub fn native(&mut self) {
    self.native = true;
  }

  pub fn selection(&self) -> Option<std::ops::Range<usize>> {
    let anchor = self.anchor?;
    (anchor != self.cursor)
      .then_some(anchor.min(self.cursor)..anchor.max(self.cursor))
  }

  pub fn can_undo(&self) -> bool {
    !self.undo.is_empty()
  }

  pub fn selected(&self) -> Option<&str> {
    if self.kind == PromptKind::Password {
      return None;
    }
    self.selection().map(|range| &self.text[range])
  }

  fn delete_selection(&mut self) -> bool {
    let range = self.selection();
    self.anchor = None;
    if let Some(range) = range {
      self.text.replace_range(range.clone(), "");
      self.cursor = range.start;
      return true;
    }
    false
  }

  fn native_key(
    &mut self,
    key: KeyEvent,
    history: &History,
  ) -> Option<PromptEvent> {
    let mods = key.modifiers;
    let base = mods - KeyModifiers::SHIFT;
    if base == KeyModifiers::SUPER {
      match key.code {
        KeyCode::Char('a') => {
          self.anchor = Some(0);
          self.cursor = self.text.len();
          self.ghost.clear();
          return Some(PromptEvent::Pending);
        }
        KeyCode::Char('x') if self.kind != PromptKind::Password => {
          self.snapshot();
          return Some(if self.delete_selection() {
            PromptEvent::Edited
          } else {
            PromptEvent::Pending
          });
        }
        KeyCode::Char('z') => {
          return Some(self.action(PromptAction::Undo, history));
        }
        _ => {}
      }
    }
    let moving = matches!(
      key.code,
      KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End
    );
    if !moving
      || !matches!(
        base,
        KeyModifiers::NONE | KeyModifiers::ALT | KeyModifiers::SUPER
      )
    {
      return None;
    }
    if mods == KeyModifiers::NONE && self.selection().is_none() {
      return None;
    }
    let old = self.cursor;
    let forward = matches!(key.code, KeyCode::Right | KeyCode::End);
    if base == KeyModifiers::SUPER
      || matches!(key.code, KeyCode::Home | KeyCode::End)
    {
      self.cursor = if forward { self.text.len() } else { 0 };
    } else if base == KeyModifiers::ALT {
      self.cursor = if forward {
        line::next_word(&self.text, self.cursor, NARROW)
      } else {
        line::word_start(&self.text, self.cursor, NARROW)
      };
    } else if !mods.contains(KeyModifiers::SHIFT) && self.selection().is_some()
    {
      let range = self.selection().unwrap();
      self.cursor = if forward { range.end } else { range.start };
    } else {
      self.step(if forward { 1 } else { -1 });
    }
    if mods.contains(KeyModifiers::SHIFT) {
      self.anchor.get_or_insert(old);
    } else {
      self.anchor = None;
    }
    self.menu = None;
    self.ghost.clear();
    Some(PromptEvent::Pending)
  }

  /// At the end of the line, also takes the suggestion's next word.
  fn forward_word(&mut self, chars: &str) {
    if self.cursor == self.text.len() && !self.ghost.is_empty() {
      let end = line::next_word(&self.ghost, 0, chars).max(1);
      let end =
        (1..=end).rev().find(|i| self.ghost.is_char_boundary(*i)).unwrap_or(0);
      let taken = self.ghost[..end].to_owned();
      self.snapshot();
      self.insert(&taken);
      return;
    }
    self.cursor = line::next_word(&self.text, self.cursor, chars);
  }

  fn edit(&mut self, change: impl FnOnce(&mut Self)) {
    self.snapshot();
    change(self);
  }

  /// Deletes `range`, keeping multi-character kills for Ctrl-y.
  fn remove(&mut self, range: std::ops::Range<usize>) {
    if range.is_empty() {
      return;
    }
    let removed: String = self.text.drain(range.clone()).collect();
    if removed.chars().count() > 1 {
      self.kill = removed;
    }
    if self.cursor > range.start {
      self.cursor = range.start.max(self.cursor.saturating_sub(range.len()));
    }
  }

  fn next_char(&self) -> usize {
    self.text[self.cursor..]
      .chars()
      .next()
      .map_or(self.cursor, |ch| self.cursor + ch.len_utf8())
  }

  fn delete_back(&mut self) {
    if let Some(ch) = self.before_cursor().chars().next_back() {
      self.cursor -= ch.len_utf8();
      self.text.remove(self.cursor);
    }
  }

  /// Ctrl-t: swaps the characters around the cursor.
  fn transpose(&mut self) {
    let at = if self.cursor == self.text.len() {
      self
        .before_cursor()
        .chars()
        .next_back()
        .map_or(0, |c| self.cursor - c.len_utf8())
    } else {
      self.cursor
    };
    let Some(before) = self.text[..at].chars().next_back() else { return };
    let Some(after) = self.text[at..].chars().next() else { return };
    let start = at - before.len_utf8();
    let end = at + after.len_utf8();
    self.text.replace_range(start..end, &format!("{after}{before}"));
    self.cursor = end;
  }

  fn restore(&mut self) -> PromptEvent {
    let Some((text, cursor)) = self.undo.pop() else {
      return PromptEvent::Pending;
    };
    self.text = text;
    self.cursor = cursor;
    self.anchor = None;
    PromptEvent::Edited
  }

  fn step(&mut self, direction: i32) {
    if direction < 0 {
      if let Some(ch) = self.before_cursor().chars().next_back() {
        self.cursor -= ch.len_utf8();
      }
    } else {
      self.cursor = self.next_char();
    }
  }

  /// Takes the whole suggestion; false when there is none to take.
  fn accept(&mut self) -> bool {
    if self.cursor != self.text.len() || self.ghost.is_empty() {
      return false;
    }
    let ghost = std::mem::take(&mut self.ghost);
    self.snapshot();
    self.insert(&ghost);
    true
  }

  /// Newest history entry extending the line, shown after the cursor.
  fn suggest(&mut self, history: &History) {
    self.ghost.clear();
    if self.text.is_empty()
      || self.cursor != self.text.len()
      || self.recall.is_some()
    {
      return;
    }
    if let Some(entry) = history.entries(self.kind).iter().rev().find(|entry| {
      entry.len() > self.text.len() && entry.starts_with(&self.text)
    }) {
      self.ghost = entry[self.text.len()..].to_owned();
    }
  }

  /// Up and Down walk history like zsh; Ctrl-r and Ctrl-s walk entries
  /// that contain the text typed before searching.
  fn recall(&mut self, history: &History, direction: i32, search: bool) {
    let entries = history.entries(self.kind);
    let recall = self.recall.get_or_insert_with(|| Recall {
      index: entries.len(),
      saved: self.text.clone(),
      pattern: None,
    });
    if search && recall.pattern.is_none() {
      recall.pattern = Some(recall.saved.clone());
    }
    let matches = |entry: &String| {
      recall
        .pattern
        .as_ref()
        .is_none_or(|pattern| entry.contains(pattern.as_str()))
    };
    let found = if direction < 0 {
      entries[..recall.index.min(entries.len())].iter().rposition(matches)
    } else {
      entries
        .iter()
        .skip(recall.index + 1)
        .position(matches)
        .map(|offset| recall.index + 1 + offset)
    };
    match found {
      Some(index) => {
        recall.index = index;
        self.text.clone_from(&entries[index]);
      }
      None if direction > 0 => {
        recall.index = entries.len();
        self.text.clone_from(&recall.saved);
      }
      None => {}
    }
    self.cursor = self.text.len();
  }

  /// First Tab lists matches and inserts their common prefix, like zsh;
  /// later Tabs cycle through them.
  fn cycle(&mut self, forward: bool) -> PromptEvent {
    let Some(menu) = &mut self.menu else {
      return if forward {
        PromptEvent::Complete
      } else {
        PromptEvent::Pending
      };
    };
    let count = menu.items.len();
    menu.selected = Some(match menu.selected {
      None if forward => 0,
      None => count - 1,
      Some(index) if forward => (index + 1) % count,
      Some(index) => (index + count - 1) % count,
    });
    self.apply();
    PromptEvent::Edited
  }

  /// Picks completion `index`, as Tab would.
  pub fn pick(&mut self, index: usize) {
    let Some(menu) = &mut self.menu else { return };
    if index < menu.items.len() {
      menu.selected = Some(index);
      self.apply();
    }
  }

  fn completion_hint(&self, selected: bool) -> String {
    let mut hints =
      vec![if selected { "Selected" } else { "Unselected" }.into()];
    for (action, label) in [
      (PromptAction::Complete { forward: true }, "select"),
      (PromptAction::Submit, if selected { "open" } else { "submit query" }),
      (PromptAction::Cancel, "cancel"),
    ] {
      let key = self.bindings.shortcut(&Target::Prompt(action));
      if !key.is_empty() {
        hints.push(format!("{key} {label}"));
      }
    }
    hints.join("  ")
  }

  pub fn describe(&mut self, label: impl Fn(&str) -> String) {
    if let Some(menu) = &mut self.menu {
      menu.labels = menu.items.iter().map(|item| label(item)).collect();
      menu.guidance = true;
    }
  }

  pub fn choices(&mut self, start: usize, items: Vec<String>) {
    let original = self.text.clone();
    let cursor = self.cursor;
    self.complete(start, items);
    self.text = original;
    self.cursor = cursor;
  }

  /// Offers `items` for the text from byte `start` to the cursor.
  pub fn complete(&mut self, start: usize, items: Vec<String>) {
    if items.is_empty() || start > self.cursor {
      return;
    }
    self.snapshot();
    self.ghost.clear();
    let common = common_prefix(&items);
    let single = items.len() == 1;
    let labels = items
      .iter()
      .map(|item| {
        let start = item.trim_end_matches('/').rfind('/').map_or(0, |i| i + 1);
        item[start..].to_owned()
      })
      .collect();
    self.menu = Some(Menu {
      items,
      labels,
      selected: None,
      start,
      end: self.cursor,
      original: self.text.clone(),
      guidance: false,
    });
    if single {
      if let Some(menu) = &mut self.menu {
        menu.selected = Some(0);
      }
      self.apply();
      self.menu = None;
    } else if common.len() > self.cursor - start {
      self.text.replace_range(start..self.cursor, &common);
      self.cursor = start + common.len();
    }
  }

  fn apply(&mut self) {
    let Some(menu) = &self.menu else { return };
    let Some(item) = menu.selected.map(|index| &menu.items[index]) else {
      return;
    };
    self.text = format!(
      "{}{item}{}",
      &menu.original[..menu.start],
      &menu.original[menu.end..]
    );
    self.cursor = menu.start + item.len();
  }
}

fn common_prefix(items: &[String]) -> String {
  let Some(first) = items.first() else { return String::new() };
  let mut end = first.len();
  for item in &items[1..] {
    end = first
      .char_indices()
      .zip(item.chars())
      .find(|((_, a), b)| a != b)
      .map_or(end.min(item.len()), |((index, _), _)| index)
      .min(end);
  }
  first[..end].to_owned()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn drawing_positions_the_wide_text_cursor() {
    let mut prompt = Prompt::new(PromptKind::Command);
    prompt.replace("界a");
    let screen = crate::testing::screen::cells(10, 2);
    let at = At { column: 7, row: 3 };
    let mut scene = vpdf_model::scene::Scene::new();
    prompt.scene(&mut scene, screen, at, None);
    assert_eq!(crate::testing::text(&scene), ":界a      ");
    assert!(scene.contains(&Op::Move { row: 4, column: 11 }));
    assert!(scene.ends_with(&[Op::BlockCursor, Op::ShowCursor]));
    for screen in
      [Screen { rows: 0, ..screen }, Screen { columns: 0, ..screen }]
    {
      scene.clear();
      prompt.scene(&mut scene, screen, at, None);
      assert_eq!(scene, [] as [Op<'_>; 0]);
    }
  }

  fn press(
    prompt: &mut Prompt,
    history: &History,
    keys: &[(KeyCode, KeyModifiers)],
  ) {
    for (code, modifiers) in keys {
      prompt.feed(KeyEvent::new(*code, *modifiers), history);
    }
  }

  const NONE: KeyModifiers = KeyModifiers::NONE;
  const CTRL: KeyModifiers = KeyModifiers::CONTROL;
  const ALT: KeyModifiers = KeyModifiers::ALT;

  fn typed(text: &str, history: &History) -> Prompt {
    let mut prompt = Prompt::new(PromptKind::Command);
    for ch in text.chars() {
      prompt.feed(KeyEvent::new(KeyCode::Char(ch), NONE), history);
    }
    prompt
  }

  #[test]
  fn control_u_preserves_suffix() {
    let history = History::default();
    for (text, cursor, suffix) in [
      ("edit file.pdf", 5, "file.pdf"),
      ("edit café.pdf", 10, ".pdf"),
      ("abc", 0, "abc"),
    ] {
      let mut prompt = typed(text, &history);
      prompt.cursor = cursor;
      press(&mut prompt, &history, &[(KeyCode::Char('u'), CTRL)]);
      assert_eq!(prompt.text, suffix);
      assert_eq!(prompt.cursor, 0);
      if cursor > 0 {
        press(&mut prompt, &history, &[(KeyCode::Char('_'), CTRL)]);
        assert_eq!(prompt.text, text);
        assert_eq!(prompt.cursor, cursor);
      }
    }
  }

  #[test]
  fn emacs_editing_keys() {
    let history = History::default();
    let mut prompt = typed("e ~/papers/attention.pdf", &history);
    press(&mut prompt, &history, &[(KeyCode::Char('w'), CTRL)]);
    assert_eq!(prompt.text, "e ~/papers/attention.");
    press(&mut prompt, &history, &[(KeyCode::Backspace, ALT)]);
    assert_eq!(prompt.text, "e ");
    press(&mut prompt, &history, &[(KeyCode::Char('y'), CTRL)]);
    assert_eq!(prompt.text, "e ~/papers/attention.");
    press(&mut prompt, &history, &[(KeyCode::Char('_'), CTRL)]);
    assert_eq!(prompt.text, "e ");
    press(&mut prompt, &history, &[(KeyCode::Char('u'), CTRL)]);
    assert_eq!(prompt.text, "");
    let mut prompt = typed("pgae", &history);
    let back = (KeyCode::Char('b'), CTRL);
    press(&mut prompt, &history, &[back, back, (KeyCode::Char('t'), CTRL)]);
    assert_eq!(prompt.text, "page");
    press(
      &mut prompt,
      &history,
      &[(KeyCode::Char('a'), CTRL), (KeyCode::Char('k'), CTRL)],
    );
    assert_eq!((prompt.text.as_str(), prompt.display().2), ("", 1));
  }

  #[test]
  fn suggestions_come_from_history() {
    let mut history = History::default();
    for entry in ["ocr deu", "page 4", "ocr eng"] {
      history.push(PromptKind::Command, entry);
    }
    let mut prompt = typed("oc", &history);
    assert_eq!(prompt.ghost, "r eng");
    press(&mut prompt, &history, &[(KeyCode::Char('f'), ALT)]);
    assert_eq!(prompt.text, "ocr ");
    press(&mut prompt, &history, &[(KeyCode::Char(' '), CTRL)]);
    assert_eq!((prompt.text.as_str(), prompt.ghost.as_str()), ("ocr eng", ""));
  }

  #[test]
  fn password_prompts_skip_the_editor() {
    let history = History::default();
    let mut prompt = Prompt::new(PromptKind::Password);
    press(&mut prompt, &history, &[(KeyCode::Char('x'), CTRL)]);
    let event = prompt.feed(KeyEvent::new(KeyCode::Char('e'), CTRL), &history);
    assert_ne!(event, PromptEvent::Editor);
    assert_eq!(prompt.paste("secret"), PromptEvent::Edited);
    assert_eq!(prompt.text, "secret");
  }

  #[test]
  fn history_walks_like_zsh() {
    let mut history = History::default();
    for entry in ["ocr deu", "page 4", "ocr eng", "page 4"] {
      history.push(PromptKind::Command, entry);
    }
    assert_eq!(history.commands, ["ocr deu", "ocr eng", "page 4"]);
    let mut prompt = typed("x", &history);
    press(&mut prompt, &history, &[(KeyCode::Up, NONE)]);
    assert_eq!(prompt.text, "page 4");
    press(&mut prompt, &history, &[(KeyCode::Down, NONE)]);
    assert_eq!(prompt.text, "x");
    let mut prompt = typed("deu", &history);
    press(&mut prompt, &history, &[(KeyCode::Char('r'), CTRL)]);
    assert_eq!(prompt.text, "ocr deu");
  }

  #[test]
  fn completion_lists_then_cycles() {
    let history = History::default();
    let mut prompt = typed("t", &history);
    let tab = KeyEvent::new(KeyCode::Tab, NONE);
    assert_eq!(prompt.feed(tab, &history), PromptEvent::Complete);
    prompt.complete(0, vec!["tabedit".into(), "tabnext".into(), "text".into()]);
    assert_eq!(prompt.text, "t");
    prompt.complete(0, vec!["tabedit".into(), "tabnext".into()]);
    assert_eq!(prompt.text, "tab");
    assert_eq!(prompt.menu.as_ref().map(|m| m.selected), Some(None));
    press(&mut prompt, &history, &[(KeyCode::Tab, NONE), (KeyCode::Tab, NONE)]);
    assert_eq!(prompt.text, "tabnext");
    press(
      &mut prompt,
      &history,
      &[(KeyCode::BackTab, NONE), (KeyCode::Char(' '), NONE)],
    );
    assert_eq!(prompt.text, "tabedit ");
    assert!(prompt.menu.is_none());
  }

  #[test]
  fn chords_match_legacy_keys() {
    let history = History::default();
    for (ch, modifiers, want) in [
      ('[', CTRL, PromptEvent::Cancel),
      ('m', CTRL, PromptEvent::Submit),
      ('i', CTRL, PromptEvent::Pending),
      ('a', KeyModifiers::SUPER, PromptEvent::Pending),
      ('a', KeyModifiers::HYPER, PromptEvent::Pending),
      ('a', KeyModifiers::META, PromptEvent::Pending),
      ('u', CTRL | KeyModifiers::SUPER, PromptEvent::Pending),
      ('m', CTRL | KeyModifiers::SUPER, PromptEvent::Pending),
    ] {
      let mut prompt = typed("t", &history);
      let key = KeyEvent::new(KeyCode::Char(ch), modifiers);
      assert_eq!(prompt.feed(key, &history), want, "{modifiers:?}-{ch}");
    }
    let mut prompt = typed("t", &history);
    prompt.feed(KeyEvent::new(KeyCode::Char('x'), CTRL), &history);
    let super_e = KeyEvent::new(KeyCode::Char('e'), KeyModifiers::SUPER);
    assert_eq!(prompt.feed(super_e, &history), PromptEvent::Pending);
    let e = KeyEvent::new(KeyCode::Char('e'), NONE);
    assert_eq!(prompt.feed(e, &history), PromptEvent::Edited, "prefix ends");
  }

  #[test]
  fn reset_preserves_text_and_caret() {
    let history = History::default();
    let mut prompt = typed("page", &history);
    press(&mut prompt, &history, &[(KeyCode::Left, NONE)]);
    prompt.feed(KeyEvent::new(KeyCode::Char('x'), CTRL), &history);
    prompt.reset_input();
    assert_eq!(prompt.text(), "page");
    prompt.feed(KeyEvent::new(KeyCode::Char('!'), NONE), &history);
    assert_eq!(prompt.text(), "pag!e");
  }

  #[test]
  fn ctrl_x_ctrl_e_opens_the_editor() {
    let history = History::default();
    let mut prompt = typed("page", &history);
    prompt.feed(KeyEvent::new(KeyCode::Char('x'), CTRL), &history);
    let event = prompt.feed(KeyEvent::new(KeyCode::Char('e'), CTRL), &history);
    assert_eq!(event, PromptEvent::Editor);
  }
}

#[cfg(test)]
mod native_tests {
  use super::*;
  fn key(prompt: &mut Prompt, code: KeyCode, modifiers: KeyModifiers) {
    prompt.feed(KeyEvent::new(code, modifiers), &History::default());
  }
  #[test]
  fn selection_replaces_and_undoes() {
    let mut p = Prompt::new(PromptKind::Command);
    p.native();
    p.paste("café 日本語");
    key(&mut p, KeyCode::Char('a'), KeyModifiers::SUPER);
    assert_eq!(p.selected(), Some("café 日本語"));
    p.paste("章");
    assert_eq!(p.text(), "章");
    key(&mut p, KeyCode::Char('z'), KeyModifiers::SUPER);
    assert_eq!(p.text(), "café 日本語");
    key(&mut p, KeyCode::Left, KeyModifiers::SHIFT);
    assert_eq!(p.selected(), Some("語"));
    key(&mut p, KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(p.text(), "café 日本");
    key(&mut p, KeyCode::Home, KeyModifiers::SHIFT);
    assert_eq!(p.selected(), Some("café 日本"));
    key(&mut p, KeyCode::Char('x'), KeyModifiers::SUPER);
    assert_eq!(p.text(), "");
    key(&mut p, KeyCode::Char('z'), KeyModifiers::SUPER);
    key(&mut p, KeyCode::Home, KeyModifiers::NONE);
    key(&mut p, KeyCode::Right, KeyModifiers::ALT | KeyModifiers::SHIFT);
    assert_eq!(p.selected(), Some("café "));
    key(&mut p, KeyCode::Char('!'), KeyModifiers::NONE);
    assert_eq!(p.text(), "!日本");
  }
  #[test]
  fn passwords_never_copy_or_enter_history() {
    let mut p = Prompt::new(PromptKind::Password);
    p.native();
    p.paste("secret");
    key(&mut p, KeyCode::Char('a'), KeyModifiers::SUPER);
    assert!(p.selected().is_none());
    key(&mut p, KeyCode::Char('x'), KeyModifiers::SUPER);
    assert_eq!(p.text(), "secret");
    key(&mut p, KeyCode::Delete, KeyModifiers::NONE);
    assert_eq!(p.text(), "");
    let mut history = History::default();
    history.push(PromptKind::Password, "secret");
    assert_eq!(history.commands(), [] as [String; 0]);
    assert_eq!(history.searches(), [] as [String; 0]);
  }
}
