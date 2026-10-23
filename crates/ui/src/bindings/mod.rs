pub mod actions;
mod defaults;
mod display;
pub mod window;
pub use defaults::Bindings;
mod target;
pub use target::{Control, HintAction, Target};
mod prompt;
pub use prompt::PromptAction;
mod context;
pub use context::Context;
mod sequence;
pub use sequence::Sequence;
mod snapshot;
use crate::reference::Source;
pub use snapshot::{Binding, Snapshot, Unbound};
use std::collections::BTreeMap;
use vpdf_model::input::{KeyEvent, KeyEventKind};

#[derive(Clone, Debug)]
pub struct Map<T> {
  entries: Vec<(Sequence, T)>,
  disabled: Vec<Sequence>,
  sources: BTreeMap<String, Source>,
  known: Vec<Sequence>,
}

// Provenance does not change dispatch.
impl<T: PartialEq> PartialEq for Map<T> {
  fn eq(&self, other: &Self) -> bool {
    self.entries == other.entries && self.disabled == other.disabled
  }
}

impl<T: Eq> Eq for Map<T> {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change<T> {
  Bind(T),
  Unbind,
  Invalid(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit<T> {
  pub sequence: Sequence,
  pub change: Change<T>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Diagnostic {
  pub sequence: String,
  pub reason: String,
}

impl<T> Map<T> {
  pub fn new(mut entries: Vec<(Sequence, T)>) -> crate::Result<Self> {
    entries.sort_by_key(|(sequence, _)| sequence.to_string());
    let map = Self {
      entries,
      disabled: Vec::new(),
      sources: BTreeMap::new(),
      known: Vec::new(),
    };
    anyhow::ensure!(map.conflicts().is_empty(), "Ambiguous binding map");
    Ok(map)
  }

  pub fn entries(&self) -> impl Iterator<Item = (&Sequence, &T)> {
    self.entries.iter().map(|(sequence, target)| (sequence, target))
  }

  pub fn get(&self, sequence: &Sequence) -> Option<&T> {
    self
      .entries
      .iter()
      .find(|(key, _)| key == sequence)
      .map(|(_, target)| target)
  }

  pub fn is_unbound(&self, sequence: &Sequence) -> bool {
    self.disabled.contains(sequence)
  }

  pub fn unbound(&self) -> impl Iterator<Item = &Sequence> {
    self.disabled.iter()
  }

  pub fn source(&self, sequence: &Sequence) -> Source {
    self.sources.get(&sequence.to_string()).cloned().unwrap_or_default()
  }

  pub fn known(&self) -> impl Iterator<Item = &Sequence> {
    self
      .known
      .iter()
      .chain(self.disabled.iter())
      .chain(self.entries.iter().map(|(sequence, _)| sequence))
  }

  pub fn remember(&mut self, previous: &Self) {
    self.recognize(previous.known().cloned());
  }

  pub(crate) fn recognize(
    &mut self,
    sequences: impl IntoIterator<Item = Sequence>,
  ) {
    self.known.extend(sequences);
    self.known.sort_by_key(ToString::to_string);
    self.known.dedup();
  }

  fn conflicts(&self) -> Vec<(Sequence, Sequence)> {
    let mut conflicts = Vec::new();
    for (index, (left, _)) in self.entries.iter().enumerate() {
      for (right, _) in &self.entries[index + 1..] {
        if left.starts_with(right.keys()) || right.starts_with(left.keys()) {
          conflicts.push((left.clone(), right.clone()));
        }
      }
    }
    conflicts
      .sort_by_key(|(left, right)| (left.to_string(), right.to_string()));
    conflicts
  }
}

impl<T: Clone> Map<T> {
  fn replace(&mut self, sequence: &Sequence, target: Option<&T>) {
    self.entries.retain(|(key, _)| key != sequence);
    self.disabled.retain(|key| key != sequence);
    self.sources.remove(&sequence.to_string());
    if let Some(target) = target {
      self.entries.push((sequence.clone(), target.clone()));
    }
  }

  fn restore(&mut self, sequence: &Sequence, previous: &Self) {
    self.replace(sequence, previous.get(sequence));
    self.sources.insert(sequence.to_string(), previous.source(sequence));
    if previous.is_unbound(sequence) {
      self.disabled.push(sequence.clone());
    }
  }

  pub fn apply(
    &self,
    previous: &Self,
    edits: &[Edit<T>],
  ) -> (Self, Vec<Diagnostic>) {
    self.apply_sourced(previous, edits, |_| Source::Default)
  }

  pub fn apply_sourced(
    &self,
    previous: &Self,
    edits: &[Edit<T>],
    source: impl Fn(&Sequence) -> Source,
  ) -> (Self, Vec<Diagnostic>) {
    let mut candidate = self.clone();
    if edits.is_empty() {
      candidate.remember(previous);
      return (candidate, Vec::new());
    }
    candidate.known.extend(previous.known().cloned());
    candidate.known.extend(edits.iter().map(|edit| edit.sequence.clone()));
    let mut diagnostics = Vec::new();
    let mut ordered: Vec<_> = edits.iter().collect();
    ordered.sort_by_key(|edit| edit.sequence.to_string());
    for edit in &ordered {
      let duplicate =
        ordered.iter().filter(|other| other.sequence == edit.sequence).count()
          > 1;
      let target = if duplicate {
        diagnostics.push(Diagnostic {
          sequence: edit.sequence.to_string(),
          reason: "Duplicate binding identity".into(),
        });
        previous.get(&edit.sequence)
      } else {
        match &edit.change {
          Change::Bind(target) => Some(target),
          Change::Unbind => None,
          Change::Invalid(reason) => {
            diagnostics.push(Diagnostic {
              sequence: edit.sequence.to_string(),
              reason: reason.clone(),
            });
            previous.get(&edit.sequence)
          }
        }
      };
      candidate.replace(&edit.sequence, target);
      let origin = if duplicate || matches!(&edit.change, Change::Invalid(_)) {
        previous.source(&edit.sequence)
      } else {
        source(&edit.sequence)
      };
      candidate.sources.insert(edit.sequence.to_string(), origin);
      let disabled = if duplicate || matches!(&edit.change, Change::Invalid(_))
      {
        previous.is_unbound(&edit.sequence)
      } else {
        matches!(&edit.change, Change::Unbind)
      };
      if disabled {
        candidate.disabled.push(edit.sequence.clone());
      }
    }
    let conflicts = candidate.conflicts();
    for (left, right) in conflicts {
      for (sequence, other) in [(&left, &right), (&right, &left)] {
        if ordered.iter().any(|edit| edit.sequence == *sequence) {
          diagnostics.push(Diagnostic {
            sequence: sequence.to_string(),
            reason: format!("Prefix conflict with {other}"),
          });
          candidate.restore(sequence, previous);
        }
      }
    }
    if !candidate.conflicts().is_empty() {
      diagnostics.push(Diagnostic {
        sequence: String::new(),
        reason: "Ambiguous recovery; previous context retained".into(),
      });
      candidate = previous.clone();
    }
    candidate.disabled.sort_by_key(ToString::to_string);
    candidate.disabled.dedup();
    candidate.entries.sort_by_key(|(sequence, _)| sequence.to_string());
    candidate.known.sort_by_key(ToString::to_string);
    candidate.known.dedup();
    diagnostics.sort_by(|left, right| {
      (&left.sequence, &left.reason).cmp(&(&right.sequence, &right.reason))
    });
    diagnostics.dedup();
    (candidate, diagnostics)
  }
}

impl<T: Clone + PartialEq> Map<T> {
  pub fn changes_from(&self, defaults: &Self) -> Vec<Edit<T>> {
    let mut edits = Vec::new();
    for (sequence, target) in self.entries() {
      if defaults.get(sequence) != Some(target) {
        edits.push(Edit {
          sequence: sequence.clone(),
          change: Change::Bind(target.clone()),
        });
      }
    }
    for (sequence, _) in defaults.entries() {
      if self.get(sequence).is_none() {
        edits.push(Edit { sequence: sequence.clone(), change: Change::Unbind });
      }
    }
    for sequence in self.unbound() {
      if !edits.iter().any(|edit| edit.sequence == *sequence) {
        edits.push(Edit { sequence: sequence.clone(), change: Change::Unbind });
      }
    }
    edits
  }
}

#[derive(Clone, Debug, Default)]
pub struct Pending {
  keys: Vec<KeyEvent>,
  cancelling: Vec<KeyEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Match<T> {
  Pending,
  Target(T),
  Unbound,
  Consumed,
}

impl Pending {
  pub fn clear(&mut self) {
    self.keys.clear();
    self.cancelling.clear();
  }

  pub fn keys(&self) -> &[KeyEvent] {
    &self.keys
  }

  pub fn feed_cancellable<T: Clone>(
    &mut self,
    map: &Map<T>,
    key: KeyEvent,
    cancels: impl Fn(&T) -> bool,
  ) -> Match<T> {
    if key.kind == KeyEventKind::Release {
      return Match::Consumed;
    }
    if self.keys.is_empty() {
      return self.feed(map, key);
    }
    if self.cancelling.is_empty() {
      let mut continuation = self.keys.clone();
      continuation.push(sequence::canonical(key));
      if map
        .entries
        .iter()
        .any(|(sequence, _)| sequence.starts_with(&continuation))
      {
        return self.feed(map, key);
      }
    }
    let mut trial = self.cancelling.clone();
    trial.push(sequence::canonical(key));
    for (sequence, target) in &map.entries {
      if !cancels(target) || !sequence.starts_with(&trial) {
        continue;
      }
      if sequence.keys() == trial {
        let target = target.clone();
        self.clear();
        return Match::Target(target);
      }
      self.cancelling = trial;
      return Match::Pending;
    }
    if !self.cancelling.is_empty() {
      self.clear();
      return Match::Consumed;
    }
    self.feed(map, key)
  }

  pub fn feed<T: Clone>(&mut self, map: &Map<T>, key: KeyEvent) -> Match<T> {
    if key.kind == KeyEventKind::Release {
      return Match::Consumed;
    }
    let continuing = !self.keys.is_empty();
    self.keys.push(sequence::canonical(key));
    for (sequence, target) in &map.entries {
      if sequence.keys() == self.keys {
        let target = target.clone();
        self.clear();
        return Match::Target(target);
      }
    }
    if map.entries.iter().any(|(sequence, _)| sequence.starts_with(&self.keys))
    {
      return Match::Pending;
    }
    self.clear();
    if continuing { Match::Consumed } else { Match::Unbound }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn sequence(text: &str) -> Sequence {
    Sequence::parse(text).unwrap()
  }

  fn map(entries: &[(&str, u8)]) -> Map<u8> {
    Map::new(
      entries.iter().map(|(key, value)| (sequence(key), *value)).collect(),
    )
    .unwrap()
  }

  fn edit(key: &str, change: Change<u8>) -> Edit<u8> {
    Edit { sequence: sequence(key), change }
  }

  #[test]
  fn rejects_ambiguous_maps() {
    assert!(Map::new(vec![(sequence("g"), 1), (sequence("gg"), 2)]).is_err());
    assert!(
      Map::new(vec![(sequence("<Enter>"), 1), (sequence("<CR>"), 2)]).is_err()
    );
    assert!(Map::new(vec![(sequence("gg"), 1), (sequence("gt"), 2)]).is_ok());
  }

  #[test]
  fn edits_are_simultaneous() {
    let defaults = map(&[("gg", 1), ("gt", 2), ("q", 3)]);
    let edits = [
      edit("g", Change::Bind(4)),
      edit("gg", Change::Unbind),
      edit("gt", Change::Unbind),
    ];
    let (next, warnings) = defaults.apply(&defaults, &edits);
    assert_eq!(warnings, Vec::<Diagnostic>::new());
    assert_eq!(next.get(&sequence("g")), Some(&4));
    assert!(next.get(&sequence("gg")).is_none());
    let (reset, warnings) = defaults.apply(&next, &[]);
    assert_eq!(warnings, Vec::<Diagnostic>::new());
    assert_eq!(reset, defaults);
  }

  #[test]
  fn empty_edits_keep_history() {
    let defaults = map(&[("q", 1)]);
    let previous = map(&[("q", 1), ("xy", 2)]);
    let (next, warnings) = defaults
      .apply_sourced(&previous, &[], |_| panic!("Empty edits have no source"));
    assert_eq!(next, defaults);
    assert_eq!(warnings, Vec::<Diagnostic>::new());
    assert!(next.known().any(|key| *key == sequence("xy")));
  }

  #[test]
  fn conflicts_retain_previous_and_apply_unrelated() {
    let defaults = map(&[("gg", 1), ("gt", 2), ("q", 3)]);
    let previous = map(&[("gg", 5), ("gt", 2), ("q", 3)]);
    let edits = [edit("g", Change::Bind(4)), edit("q", Change::Bind(6))];
    let (next, warnings) = defaults.apply(&previous, &edits);
    assert_eq!(next.get(&sequence("q")), Some(&6));
    assert!(next.get(&sequence("g")).is_none());
    assert_eq!(next.get(&sequence("gg")), Some(&1));
    assert!(
      warnings.iter().any(|d| d.sequence == "g" && d.reason.contains("gg"))
    );
    let reversed: Vec<_> = edits.into_iter().rev().collect();
    assert_eq!(defaults.apply(&previous, &reversed), (next, warnings));
  }

  #[test]
  fn conflicting_user_edits_revert_together() {
    let defaults = map(&[("q", 1)]);
    let edits = [
      edit("g", Change::Bind(2)),
      edit("gg", Change::Bind(3)),
      edit("q", Change::Bind(4)),
    ];
    let (next, warnings) = defaults.apply(&defaults, &edits);
    assert_eq!(next, map(&[("q", 4)]));
    assert_eq!(warnings.len(), 2);
  }

  #[test]
  fn invalid_entries_keep_previous_unbinding() {
    let defaults = map(&[("q", 1), ("j", 2)]);
    let previous = map(&[("j", 4)]);
    let (next, warnings) = defaults.apply(
      &previous,
      &[
        edit("q", Change::Invalid("Unknown action".into())),
        edit("j", Change::Invalid("Wrong argument".into())),
      ],
    );
    assert_eq!(next, previous);
    assert_eq!(warnings.len(), 2);
  }

  #[test]
  fn ambiguous_recovery_retains_context() {
    let defaults = map(&[("ga", 1), ("q", 2)]);
    let previous = map(&[("g", 3), ("q", 2)]);
    let edits = [
      edit("g", Change::Invalid("Unknown action".into())),
      edit("q", Change::Bind(5)),
    ];
    let (next, warnings) = defaults.apply(&previous, &edits);
    assert!(warnings.iter().any(|d| d.reason.contains("Ambiguous recovery")));
    assert_eq!(next, previous);
  }

  #[test]
  fn duplicate_aliases_retain_previous() {
    let defaults = map(&[("<CR>", 1), ("q", 2)]);
    let previous = map(&[("<CR>", 3), ("q", 2)]);
    let edits = [
      edit("<Enter>", Change::Bind(4)),
      edit("<CR>", Change::Unbind),
      edit("q", Change::Bind(5)),
    ];
    let (next, warnings) = defaults.apply(&previous, &edits);
    assert_eq!(next, map(&[("<CR>", 3), ("q", 5)]));
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].sequence, "<CR>");
  }

  #[test]
  fn continuation_waits_then_consumes_errors() {
    let map = map(&[("gg", 1), ("gt", 2), ("q", 3)]);
    let mut pending = Pending::default();
    let key = |text| sequence(text).keys()[0];
    assert_eq!(pending.feed(&map, key("g")), Match::Pending);
    assert_eq!(pending.feed(&map, key("q")), Match::Consumed);
    assert_eq!(pending.feed(&map, key("q")), Match::Target(3));
    assert_eq!(pending.feed(&map, key("x")), Match::Unbound);
    assert_eq!(pending.feed(&map, key("g")), Match::Pending);
    assert_eq!(pending.feed(&map, key("t")), Match::Target(2));
    assert_eq!(pending.keys(), []);
  }
}

#[cfg(test)]
mod cancellation_tests {
  use super::*;

  #[test]
  fn cancellation_preserves_continuations() {
    let map = Map::new(
      [("gg", false), ("gq", true), ("xy", true), ("<Esc>", true)]
        .into_iter()
        .map(|(keys, cancel)| (Sequence::parse(keys).unwrap(), cancel))
        .collect(),
    )
    .unwrap();
    for (keys, expected) in [
      ("gg", Match::Target(false)),
      ("gq", Match::Target(true)),
      ("gxy", Match::Target(true)),
      ("g<Esc>", Match::Target(true)),
      ("gxg", Match::Consumed),
    ] {
      let mut pending = Pending::default();
      let sequence = Sequence::parse(keys).unwrap();
      let (last, prefix) = sequence.keys().split_last().unwrap();
      for &key in prefix {
        assert_eq!(
          pending.feed_cancellable(&map, key, |v| *v),
          Match::Pending,
          "{keys}"
        );
      }
      assert_eq!(
        pending.feed_cancellable(&map, *last, |v| *v),
        expected,
        "{keys}"
      );
      assert!(pending.keys().is_empty(), "{keys}");
    }
  }

  #[test]
  fn cancellation_can_use_a_sequence() {
    let map = Map::new(vec![
      (Sequence::parse("gg").unwrap(), false),
      (Sequence::parse("xy").unwrap(), true),
    ])
    .unwrap();
    let mut pending = Pending::default();
    let key = |text| Sequence::parse(text).unwrap().keys()[0];
    assert_eq!(
      pending.feed_cancellable(&map, key("g"), |v| *v),
      Match::Pending
    );
    assert_eq!(
      pending.feed_cancellable(&map, key("x"), |v| *v),
      Match::Pending
    );
    assert_eq!(
      pending.feed_cancellable(&map, key("y"), |v| *v),
      Match::Target(true)
    );
    assert_eq!(pending.keys(), []);
  }
}
