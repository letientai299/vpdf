use crate::bindings::{Binding, Context, Sequence, Snapshot, Target, actions};
use std::collections::BTreeMap;
use vpdf_model::text::Position;

pub mod build;

/// ```compile_fail
/// vpdf_ui::topics::TopicId(String::from("h"));
/// ```
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TopicId(String);

impl TopicId {
  pub fn parse(name: &str) -> crate::Result<Self> {
    anyhow::ensure!(
      name.len() > 1
        && name.starts_with(|ch: char| ch.is_ascii_lowercase())
        && name
          .bytes()
          .all(|ch| ch.is_ascii_alphanumeric() || b"._-".contains(&ch)),
      "Invalid help topic ID"
    );
    Ok(Self(name.into()))
  }

  pub fn as_str(&self) -> &str {
    &self.0
  }

  pub fn parameterized(&self) -> bool {
    self.0.split('.').skip(1).any(|part| part.contains('_'))
  }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Destination {
  Authored(TopicId),
  Action(TopicId),
  Command(TopicId),
  Configuration,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaneContext {
  pub id: u32,
  pub document: String,
  pub document_generation: u64,
  pub position: Position,
  pub zoom: f32,
  pub cursorline: bool,
  pub text_mode: bool,
  pub panel: Option<crate::keys::PanelKind>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Invocation {
  Home,
  Context(Context),
  Pane { context: Context, pane: PaneContext },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
  pub configuration: crate::reference::Configuration,
  pub bindings: Snapshot,
  pub invocation: Invocation,
}

impl Invocation {
  pub const fn home() -> Self {
    Self::Home
  }

  pub const fn context(&self) -> Context {
    match self {
      Self::Home => Context::Home,
      Self::Context(context) | Self::Pane { context, .. } => *context,
    }
  }

  pub const fn pane(&self) -> Option<&PaneContext> {
    match self {
      Self::Home | Self::Context(_) => None,
      Self::Pane { pane, .. } => Some(pane),
    }
  }

  #[must_use]
  pub fn in_context(mut self, selected: Context) -> Self {
    if self.context() == selected {
      return self;
    }
    if let Self::Pane { context, .. } = &mut self {
      *context = selected;
      self
    } else {
      Self::Context(selected)
    }
  }
}

pub fn query_context(query: &str, fallback: Context) -> (Context, &str) {
  query
    .split_once(':')
    .and_then(|(context, key)| {
      Context::parse(context).map(|context| (context, key))
    })
    .unwrap_or((fallback, query))
}

fn control_prefix(query: &str) -> Option<(&str, &str)> {
  let (prefix, key) = query.split_once('-')?;
  (prefix.eq_ignore_ascii_case("c") || prefix.eq_ignore_ascii_case("ctrl"))
    .then_some((prefix, key))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Topic {
  pub id: TopicId,
  pub aliases: Vec<String>,
  pub title: String,
  pub destination: Destination,
}

pub fn authored() -> Vec<Topic> {
  [
    ("introduction", "Vpdf user manual", &["intro", "help", "helphelp"][..]),
    ("first-session", "First session", &["tutorial"][..]),
    ("navigation", "Navigation", &["reading"][..]),
    ("sample", "Practice text", &[][..]),
    ("search", "Search", &["searching"][..]),
    ("panes", "Tabs and panes", &["windows", "splits"][..]),
    (
      "keys",
      "Keys and input modes",
      &["notation", "prefixes", "count", "quickref"][..],
    ),
    ("zoom", "Zoom and narrow panes", &["text-view"][..]),
    ("regions", "Figures and regions", &["capture"][..]),
    (
      "selection",
      "Selecting and copying",
      &["copy", "clipboard", "registers"][..],
    ),
    ("links", "Links and notes", &["citations", "previews"][..]),
    ("ocr", "Scanned pages and OCR", &["scans"][..]),
    ("marks", "Marks and jump history", &["bookmarks", "history"][..]),
    (
      "configuration",
      "Configuring the reader",
      &["keybindings", "settings", "reading.zoom"][..],
    ),
    ("state", "Files and saved state", &["persistence", "cache"][..]),
    ("status", "Status and progress", &["performance"][..]),
    ("install", "Installation and setup", &["setup", "helpers"][..]),
    ("troubleshooting", "Troubleshooting", &["errors"][..]),
    ("commands", "Ex commands", &["ex"][..]),
  ]
  .into_iter()
  .map(|(name, title, aliases)| {
    let id = TopicId(name.into());
    Topic {
      id: id.clone(),
      aliases: aliases.iter().map(|s| (*s).into()).collect(),
      title: title.into(),
      destination: Destination::Authored(id),
    }
  })
  .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
  pub name: String,
  pub description: String,
  pub destination: Destination,
  pub action: Option<TopicId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
  Found(Destination),
  Ambiguous(Vec<Candidate>),
  Missing,
}

impl Lookup {
  pub fn message(&self) -> Option<&'static str> {
    match self {
      Self::Missing => Some("No help topic"),
      Self::Ambiguous(_) => Some("Ambiguous help topic"),
      Self::Found(_) => None,
    }
  }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActionReference {
  pub id: TopicId,
  pub title: String,
  pub defaults: Vec<Binding>,
  pub effective: Vec<Binding>,
}

pub struct Registry {
  topics: BTreeMap<TopicId, Topic>,
  names: BTreeMap<String, TopicId>,
  bindings: Snapshot,
}

impl Registry {
  pub fn new(authored: Vec<Topic>, bindings: Snapshot) -> crate::Result<Self> {
    let mut registry =
      Self { topics: BTreeMap::new(), names: BTreeMap::new(), bindings };
    registry.insert(Topic {
      id: TopicId::parse("config")?,
      aliases: Vec::new(),
      title: "Configuration".into(),
      destination: Destination::Configuration,
    })?;
    let mut targets: Vec<_> = actions::ACTIONS
      .iter()
      .filter_map(|spec| {
        if let actions::Kind::Fixed(action) = spec.kind {
          Some(Target::Reader(action))
        } else {
          None
        }
      })
      .collect();
    targets.extend(
      registry
        .bindings
        .defaults
        .iter()
        .chain(&registry.bindings.effective)
        .map(|row| row.target.clone()),
    );
    for target in targets {
      let id = TopicId::parse(&target.topic())?;
      if registry.topics.contains_key(&id) {
        continue;
      }
      registry.insert(Topic {
        id: id.clone(),
        aliases: Vec::new(),
        title: target.about(),
        destination: Destination::Action(id),
      })?;
    }
    for topic in authored {
      registry.insert(topic)?;
    }
    for spec in crate::commands::COMMANDS {
      let id = TopicId::parse(&spec.topic())?;
      let mut aliases = Vec::new();
      for name in std::iter::once(&spec.name).chain(spec.aliases.iter()) {
        aliases.push(format!(":{name}"));
        if !registry.names.contains_key(*name) {
          aliases.push((*name).into());
        }
      }
      registry.insert(Topic {
        id: id.clone(),
        title: format!(":{} — {}", spec.name, spec.help),
        aliases,
        destination: Destination::Command(id),
      })?;
    }
    Ok(registry)
  }

  fn insert(&mut self, topic: Topic) -> crate::Result<()> {
    anyhow::ensure!(
      !self.topics.contains_key(&topic.id),
      "Duplicate help topic ID"
    );
    for name in std::iter::once(topic.id.as_str())
      .chain(topic.aliases.iter().map(String::as_str))
    {
      anyhow::ensure!(
        !name.is_empty()
          && !name.chars().any(char::is_control)
          && !self.names.contains_key(name),
        "Duplicate help topic name"
      );
      self.names.insert(name.into(), topic.id.clone());
    }
    self.topics.insert(topic.id.clone(), topic);
    Ok(())
  }

  pub fn topics(&self) -> impl Iterator<Item = &Topic> {
    self.topics.values()
  }

  pub fn action(&self, id: &TopicId) -> Option<ActionReference> {
    let topic = self.topics.get(id)?;
    if !matches!(topic.destination, Destination::Action(_)) {
      return None;
    }
    let matching = |rows: &[Binding]| {
      rows
        .iter()
        .filter(|row| row.target.topic() == id.as_str())
        .cloned()
        .collect()
    };
    Some(ActionReference {
      id: id.clone(),
      title: topic.title.clone(),
      defaults: matching(&self.bindings.defaults),
      effective: matching(&self.bindings.effective),
    })
  }

  fn active(&self, context: Context) -> impl Iterator<Item = &Binding> {
    self
      .bindings
      .effective
      .iter()
      .filter(move |row| row.context == context && !row.is_prefix())
  }

  pub fn lookup(&self, query: &str, context: Context) -> Lookup {
    let (context, query) = query_context(query, context);
    let query = query
      .strip_prefix('\'')
      .and_then(|q| q.strip_suffix('\''))
      .unwrap_or(query);
    if let Some(id) = self.names.get(query)
      && id.as_str() == query
    {
      return Lookup::Found(self.topics[id].destination.clone());
    }
    let sequence = Sequence::parse(query).ok();
    if let Some(row) =
      self.active(context).find(|row| Some(&row.sequence) == sequence.as_ref())
    {
      return Lookup::Found(Destination::Action(TopicId(row.target.topic())));
    }
    if let Some((prefix, key)) = control_prefix(query) {
      let key = if prefix.len() > 1 { key.to_lowercase() } else { key.into() };
      return self.lookup(&format!("<C-{key}>"), context);
    }
    if self.key_query(query) {
      if self.bindings.effective.iter().any(|row| {
        row.context == context
          && row.is_prefix()
          && Some(&row.sequence) == sequence.as_ref()
      }) {
        return self.names.get("keys").map_or(Lookup::Missing, |id| {
          Lookup::Found(self.topics[id].destination.clone())
        });
      }
      return Lookup::Missing;
    }
    if let Some(id) = self.names.get(query) {
      return Lookup::Found(self.topics[id].destination.clone());
    }
    let mut candidates = self.named(query);
    let mut seen = Vec::new();
    candidates.retain(|candidate| {
      if seen.contains(&candidate.destination) {
        return false;
      }
      seen.push(candidate.destination.clone());
      true
    });
    match candidates.as_slice() {
      [only] => Lookup::Found(only.destination.clone()),
      [] => Lookup::Missing,
      _ => Lookup::Ambiguous(candidates),
    }
  }

  fn key_query(&self, query: &str) -> bool {
    Sequence::parse(query).is_ok_and(|sequence| {
      sequence.keys().len() == 1
        || query.contains('<')
        || self.bindings.recognized.contains(&sequence)
    })
  }

  fn named(&self, prefix: &str) -> Vec<Candidate> {
    self
      .names
      .iter()
      .filter(|(name, id)| {
        name.starts_with(prefix)
          && (name.as_str() == id.as_str() || !self.key_query(name))
      })
      .map(|(name, id)| Candidate {
        name: name.clone(),
        description: self.topics[id].title.clone(),
        destination: self.topics[id].destination.clone(),
        action: None,
      })
      .collect()
  }

  fn control_candidates(
    &self,
    prefix: &str,
    context: Context,
  ) -> Vec<Candidate> {
    if let Some((control, key)) = control_prefix(prefix) {
      let normalized =
        if control.len() > 1 { key.to_lowercase() } else { key.into() };
      self
        .complete(&format!("<C-{normalized}"), context)
        .into_iter()
        .filter_map(|mut candidate| {
          let suffix = candidate.name.strip_prefix("<C-")?.strip_suffix('>')?;
          if suffix.contains(['<', '>']) {
            return None;
          }
          let suffix =
            if control.len() > 1 && key.chars().any(char::is_uppercase) {
              suffix.to_uppercase()
            } else {
              suffix.into()
            };
          candidate.name = format!("{control}-{suffix}");
          Some(candidate)
        })
        .collect::<Vec<_>>()
    } else {
      Vec::new()
    }
  }

  pub fn complete(&self, prefix: &str, context: Context) -> Vec<Candidate> {
    let (selected, key) = query_context(prefix, context);
    if key != prefix {
      return self
        .complete(key, selected)
        .into_iter()
        .map(|mut candidate| {
          candidate.name = format!("{}:{}", selected.name(), candidate.name);
          candidate
        })
        .collect();
    }
    let aliases = self.control_candidates(prefix, context);
    let mut candidates: BTreeMap<_, _> = self
      .named(prefix)
      .into_iter()
      .map(|candidate| (candidate.name.clone(), candidate))
      .collect();
    // Key aliases follow the same classification as lookup.
    candidates.retain(|name, candidate| {
      matches!(self.lookup(name, context), Lookup::Found(ref destination)
        if *destination == candidate.destination)
    });
    for row in self.active(context) {
      if prefix.is_empty() && row.sequence.keys().len() > 1 {
        continue;
      }
      let name = row.sequence.to_string();
      if name != prefix && self.dispatch_alias(row) {
        continue;
      }
      if !name.starts_with(prefix) {
        continue;
      }
      if self.names.get(&name).is_some_and(|id| id.as_str() == name) {
        continue;
      }
      let id = TopicId(row.target.topic());
      candidates.insert(
        name.clone(),
        Candidate {
          name,
          description: row.purpose.clone(),
          destination: Destination::Action(id.clone()),
          action: Some(id),
        },
      );
    }
    for alias in aliases {
      if self.lookup(&alias.name, context)
        == Lookup::Found(alias.destination.clone())
      {
        candidates.entry(alias.name.clone()).or_insert(alias);
      }
    }
    let candidates: Vec<_> = candidates
      .into_values()
      .filter(|candidate| {
        if candidate.action.is_none()
          && self.names[&candidate.name].parameterized()
          && !prefix.contains('.')
        {
          return false;
        }
        !prefix.is_empty()
          || candidate.action.is_some()
          || (self
            .names
            .get(&candidate.name)
            .is_some_and(|id| id.as_str() == candidate.name)
            && !self.names[&candidate.name].parameterized())
      })
      .collect();
    rank_candidates(candidates, prefix)
  }

  fn dispatch_alias(&self, row: &Binding) -> bool {
    if row
      .sources
      .iter()
      .any(|source| *source != crate::reference::Source::Default)
    {
      return false;
    }
    self.active(row.context).any(|other| {
      other.target == row.target
        && other.sequence != row.sequence
        && other.sequence.keys().len() == row.sequence.keys().len()
        && other.sequence.keys().iter().zip(row.sequence.keys()).all(
          |(a, b)| {
            let code = match b.code {
              vpdf_model::input::KeyCode::Char(ch)
                if b
                  .modifiers
                  .contains(vpdf_model::input::KeyModifiers::CONTROL) =>
              {
                vpdf_model::input::KeyCode::Char(ch.to_ascii_lowercase())
              }
              code => code,
            };
            a.code == code
              && b.modifiers.contains(a.modifiers)
              && (!a.modifiers.is_empty()
                || !matches!(a.code, vpdf_model::input::KeyCode::Char(_)))
          },
        )
    })
  }

  pub fn suggest(&self, query: &str, context: Context) -> Vec<String> {
    if query.len() > 64 || query.chars().count() < 3 {
      return Vec::new();
    }
    let mut candidates: Vec<_> = self
      .complete("", context)
      .into_iter()
      .filter(|candidate| candidate.action.is_none())
      .map(|candidate| (edit_distance(query, &candidate.name), candidate.name))
      .filter(|(distance, _)| *distance <= 2)
      .collect();
    candidates.sort();
    candidates.into_iter().take(3).map(|(_, name)| name).collect()
  }
}

fn rank_candidates(
  mut candidates: Vec<Candidate>,
  prefix: &str,
) -> Vec<Candidate> {
  candidates.sort_by_key(|candidate| {
    let rank = match (&candidate.destination, &candidate.action) {
      (Destination::Authored(_), _) => 0,
      (Destination::Configuration, _) => 1,
      (_, None) => 2,
      (_, Some(_)) => 3,
    };
    let alias = match &candidate.destination {
      Destination::Authored(id) => id.as_str() != candidate.name,
      _ => false,
    };
    (
      candidate.action.is_none() || candidate.name != prefix,
      rank,
      alias,
      candidate.name.clone(),
    )
  });
  let mut guides = Vec::new();
  candidates.retain(|candidate| {
    if !matches!(candidate.destination, Destination::Authored(_)) {
      return true;
    }
    if guides.contains(&candidate.destination) {
      return false;
    }
    guides.push(candidate.destination.clone());
    true
  });
  candidates
}

fn edit_distance(query: &str, name: &str) -> usize {
  let query: Vec<_> = query.chars().collect();
  let name: Vec<_> = name.chars().collect();
  let mut rows = vec![vec![0; name.len() + 1]; query.len() + 1];
  for (i, row) in rows.iter_mut().enumerate() {
    row[0] = i;
  }
  for (j, cell) in rows[0].iter_mut().enumerate() {
    *cell = j;
  }
  for i in 1..=query.len() {
    for j in 1..=name.len() {
      rows[i][j] = (rows[i - 1][j] + 1)
        .min(rows[i][j - 1] + 1)
        .min(rows[i - 1][j - 1] + usize::from(query[i - 1] != name[j - 1]));
      if i > 1
        && j > 1
        && query[i - 1] == name[j - 2]
        && query[i - 2] == name[j - 1]
      {
        rows[i][j] = rows[i][j].min(rows[i - 2][j - 2] + 1);
      }
    }
  }
  rows[query.len()][name.len()]
}

#[cfg(test)]
mod tests;
