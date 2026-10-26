use super::{Bindings, Context, Sequence, Target};
use crate::keys::Keys;
use crate::keys::{Action, PanelKind};
use crate::reference::Source;

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
  pub context: Context,
  pub sequence: Sequence,
  pub target: Target,
  pub purpose: String,
  pub sources: Vec<Source>,
}

impl Binding {
  pub fn is_prefix(&self) -> bool {
    matches!(self.target, Target::Control(_))
  }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
  pub defaults: Vec<Binding>,
  pub effective: Vec<Binding>,
  pub recognized: Vec<Sequence>,
  pub unbound: Vec<Unbound>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unbound {
  pub context: Context,
  pub sequence: Sequence,
  pub source: Source,
}

impl Snapshot {
  #[must_use]
  pub fn in_panel(mut self, kind: PanelKind) -> Self {
    let panel = if kind == PanelKind::Recent {
      crate::panel::Panel::recent(Vec::new(), &[])
    } else {
      crate::panel::Panel::new(kind, Vec::new())
    };
    for rows in [&mut self.defaults, &mut self.effective] {
      rows.retain_mut(|row| {
        if row.context != Context::Panel
          && !(kind == PanelKind::Recent && row.context == Context::Recent)
        {
          return true;
        }
        let Target::Reader(action) = row.target else {
          return true;
        };
        if !panel.allows(action) {
          return false;
        }
        row.purpose = if action == Action::Panel(kind) {
          "close panel".into()
        } else {
          panel
            .purpose(action)
            .map_or_else(|| row.target.about(), str::to_owned)
        };
        true
      });
    }
    self
  }
}

impl Bindings {
  pub fn snapshot(&self) -> Snapshot {
    let defaults = Self::default();
    let mut snapshot = Snapshot::default();
    for &context in Context::ALL {
      snapshot.defaults.extend(Keys::references(context, &defaults));
      snapshot.effective.extend(Keys::references(context, self));
      snapshot.recognized.extend(self.map(context).known().cloned());
      snapshot.unbound.extend(self.map(context).unbound().map(|sequence| {
        Unbound {
          context,
          sequence: sequence.clone(),
          source: self.map(context).source(sequence),
        }
      }));
    }
    snapshot.recognized.extend(
      snapshot
        .defaults
        .iter()
        .chain(&snapshot.effective)
        .map(|binding| binding.sequence.clone()),
    );
    snapshot.recognized.sort_by_key(ToString::to_string);
    snapshot.recognized.dedup();
    snapshot.in_panel(PanelKind::Outline)
  }
}
