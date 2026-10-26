use super::{Heading, Outline, Target};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(
  Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize,
)]
struct Revision(u64);

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct Preferences {
  fingerprint: [u8; 32],
  closed: Vec<usize>,
  auto: bool,
  revision: Revision,
}

impl Preferences {
  pub fn newer(&self, other: &Self) -> bool {
    self.revision.0 > other.revision.0
  }
}

impl Outline {
  pub fn preferences(&self) -> Option<&Preferences> {
    self.saved.as_ref()
  }

  pub fn restore_preferences(&mut self, saved: Option<Preferences>) {
    self.saved = saved;
    self.closed.fill(false);
    self.auto = false;
    let Some(saved) = &self.saved else { return };
    self.auto = saved.auto;
    if self.headings.is_empty() {
      return;
    }
    if saved.fingerprint == self.fingerprint {
      for &source in &saved.closed {
        if let Some(node) = self.tree.nodes().get(source)
          && node.descendants > 0
        {
          self.closed[source] = true;
        }
      }
    }
    if self.auto {
      self.expand_current();
    }
  }

  pub(super) fn commit_preferences(&mut self) {
    static CLOCK: Mutex<u64> = Mutex::new(0);
    let now = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map_or(0, |time| u64::try_from(time.as_nanos()).unwrap_or(u64::MAX));
    let previous = self.saved.as_ref().map_or(0, |saved| saved.revision.0);
    let mut clock = CLOCK.lock().expect("Outline clock poisoned");
    *clock = now.max(previous.saturating_add(1)).max(clock.saturating_add(1));
    let revision = Revision(*clock);
    let mut saved = Preferences {
      fingerprint: self.fingerprint,
      closed: self
        .closed
        .iter()
        .enumerate()
        .filter_map(|(source, &closed)| closed.then_some(source))
        .collect(),
      auto: self.auto,
      revision,
    };
    // Loading preserves pending source-index choices.
    if self.headings.is_empty()
      && let Some(previous) = &self.saved
    {
      saved.fingerprint = previous.fingerprint;
      saved.closed.clone_from(&previous.closed);
    }
    self.saved = Some(saved);
  }
}

pub(super) fn fingerprint(headings: &[Heading]) -> [u8; 32] {
  let mut hash = Sha256::new();
  for heading in headings {
    hash.update((heading.depth as u64).to_le_bytes());
    hash.update((heading.title.len() as u64).to_le_bytes());
    hash.update(heading.title.as_bytes());
    match &heading.target {
      Target::Page(page) => {
        hash.update([0]);
        hash.update((page.0 as u64).to_le_bytes());
      }
      Target::Point { page, x, y } => {
        hash.update([1]);
        hash.update((page.0 as u64).to_le_bytes());
        hash.update(x.to_bits().to_le_bytes());
        hash.update(y.to_bits().to_le_bytes());
      }
      Target::Uri(uri) => {
        hash.update([2]);
        hash.update((uri.len() as u64).to_le_bytes());
        hash.update(uri.as_bytes());
      }
    }
  }
  hash.finalize().into()
}
