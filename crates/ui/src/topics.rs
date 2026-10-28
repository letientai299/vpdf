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
  Key(KeyTopic),
  Command(TopicId),
  Catalog(TopicId),
  Setting(TopicId),
  Configuration,
}

/// A key sequence matched in one context.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyTopic {
  pub action: TopicId,
  pub context: Context,
  pub sequence: Sequence,
}

impl KeyTopic {
  fn new(row: &Binding) -> Self {
    Self {
      action: TopicId(row.target.topic()),
      context: row.context,
      sequence: row.sequence.clone(),
    }
  }

  pub fn anchor(&self) -> String {
    key_anchor(self.context, &self.sequence)
  }
}

/// Names a key row within its action entry.
pub fn key_anchor(context: Context, sequence: &Sequence) -> String {
  format!(
    "{}.{}",
    context.name(),
    crate::bindings::topic_text(&sequence.to_string())
  )
}

impl Destination {
  pub const fn action(&self) -> Option<&TopicId> {
    match self {
      Self::Action(id) | Self::Key(KeyTopic { action: id, .. }) => Some(id),
      Self::Authored(_)
      | Self::Command(_)
      | Self::Catalog(_)
      | Self::Setting(_)
      | Self::Configuration => None,
    }
  }
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

fn control_notation(control: &str, key: &str) -> String {
  let chord = if control.len() > 1 { key.to_lowercase() } else { key.into() };
  if let Ok(sequence) = Sequence::parse(&format!("<C-{chord}>")) {
    return sequence.to_string();
  }
  let Some(first) = key.chars().next() else {
    return "<C-".into();
  };
  let chord = if control.len() > 1 {
    first.to_lowercase().to_string()
  } else {
    first.to_string()
  };
  format!("<C-{chord}>{}", &key[first.len_utf8()..])
}

fn completion_notation(prefix: &str) -> Option<String> {
  ["", ">", "Esc>", "-Esc>"].into_iter().find_map(|suffix| {
    let canonical =
      Sequence::parse(&format!("{prefix}{suffix}")).ok()?.to_string();
    canonical.strip_suffix(suffix).map(str::to_owned)
  })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Topic {
  pub id: TopicId,
  pub aliases: Vec<String>,
  pub title: String,
  pub destination: Destination,
}

/// The configuration action index.
pub const ACTIONS: &str = "actions";

pub fn authored() -> Vec<Topic> {
  [
    ("introduction", "Vpdf user manual", &["intro", "help", "helphelp"][..]),
    ("first-session", "First session", &["tutorial"][..]),
    ("navigation", "Navigation", &["reading"][..]),
    ("practice", "Practice text", &["sample"][..]),
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
      &["keybindings", "settings"][..],
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
    registry.catalog()?;
    Ok(registry)
  }

  /// Registers setting names; each reaches its `config` anchor.
  pub fn with_settings<'a>(
    mut self,
    settings: impl IntoIterator<Item = (&'a str, &'a str)>,
  ) -> crate::Result<Self> {
    for (name, title) in settings {
      let id = TopicId::parse(name)?;
      self.insert(Topic {
        id: id.clone(),
        aliases: Vec::new(),
        title: title.into(),
        destination: Destination::Setting(id),
      })?;
    }
    Ok(self)
  }

  /// Registers action entries after key and command names.
  fn catalog(&mut self) -> crate::Result<()> {
    let index = TopicId::parse(ACTIONS)?;
    self.insert(Topic {
      id: index.clone(),
      aliases: Vec::new(),
      title: "Actions".into(),
      destination: Destination::Catalog(index),
    })?;
    for family in crate::bindings::catalog::families()? {
      let id = TopicId::parse(&family.topic())?;
      let owner = self.names.remove(family.name);
      if let Some(owner) = owner.filter(|owner| {
        !matches!(self.topics[owner].destination, Destination::Action(_))
      }) {
        self.names.insert(family.name.into(), owner);
      }
      let aliases = if self.names.contains_key(family.name) {
        Vec::new()
      } else {
        vec![family.name.into()]
      };
      self.insert(Topic {
        id: id.clone(),
        aliases,
        title: family.purpose,
        destination: Destination::Catalog(id),
      })?;
    }
    Ok(())
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
      && (id.as_str() == query
        || matches!(self.topics[id].destination, Destination::Catalog(_)))
    {
      return Lookup::Found(self.topics[id].destination.clone());
    }
    let sequence = Sequence::parse(query).ok();
    if let Some(row) =
      self.active(context).find(|row| Some(&row.sequence) == sequence.as_ref())
    {
      return Lookup::Found(Destination::Key(KeyTopic::new(row)));
    }
    if let Some((prefix, key)) = control_prefix(query) {
      return self.lookup(&control_notation(prefix, key), context);
    }
    if let Some(sequence) = sequence.as_ref()
      && self.key_prefix(sequence, context)
    {
      if let Some(row) = self.continuations(sequence, context) {
        return Lookup::Found(Destination::Key(KeyTopic::new(row)));
      }
      let name = sequence.to_string();
      return self
        .names
        .get(&name)
        .or_else(|| self.names.get("keys"))
        .map_or(Lookup::Missing, |id| {
          Lookup::Found(self.topics[id].destination.clone())
        });
    }
    if self.key_query(query) || (query.starts_with('<') && sequence.is_none()) {
      return Lookup::Missing;
    }
    if let Some(id) = self.names.get(query) {
      return Lookup::Found(self.topics[id].destination.clone());
    }
    let mut candidates = self.named(query, context);
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

  /// First key row when one action family owns the prefix.
  fn continuations(
    &self,
    prefix: &Sequence,
    context: Context,
  ) -> Option<&Binding> {
    use crate::bindings::catalog::Family;
    let rows: Vec<_> = self
      .active(context)
      .filter(|row| {
        row.sequence.keys().len() > prefix.keys().len()
          && row.sequence.starts_with(prefix.keys())
      })
      .collect();
    let family = Family::topic_of(&rows.first()?.target);
    if rows.iter().any(|row| Family::topic_of(&row.target) != family) {
      return None;
    }
    rows.into_iter().min_by_key(|row| row.target.topic())
  }

  fn key_prefix(&self, sequence: &Sequence, context: Context) -> bool {
    self.bindings.effective.iter().any(|row| {
      row.context == context
        && ((row.is_prefix() && row.sequence == *sequence)
          || (row.sequence.keys().len() > sequence.keys().len()
            && row.sequence.starts_with(sequence.keys())))
    })
  }

  fn key_query(&self, query: &str) -> bool {
    Sequence::parse(query).is_ok_and(|sequence| {
      sequence.keys().len() == 1
        || query.contains('<')
        || self.bindings.recognized.contains(&sequence)
    })
  }

  fn named(&self, prefix: &str, context: Context) -> Vec<Candidate> {
    self
      .names
      .iter()
      .filter(|(name, id)| {
        name.starts_with(prefix)
          && (name.as_str() == id.as_str()
            || !self.key_query(name)
            || Sequence::parse(name)
              .is_ok_and(|sequence| self.key_prefix(&sequence, context)))
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
      let normalized = control_notation(control, key);
      let normalized = normalized.strip_suffix('>').unwrap_or(&normalized);
      self
        .complete(normalized, context)
        .into_iter()
        .filter_map(|mut candidate| {
          let (suffix, rest) =
            candidate.name.strip_prefix("<C-")?.split_once('>')?;
          if rest.contains(['<', '>']) {
            return None;
          }
          let suffix = if control.len() > 1
            && key.chars().next().is_some_and(char::is_uppercase)
          {
            suffix.to_uppercase()
          } else {
            suffix.into()
          };
          candidate.name = format!("{control}-{suffix}{rest}");
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
    if prefix.contains('<')
      && let Some(canonical) = completion_notation(prefix)
      && canonical != prefix
    {
      return self.complete(&canonical, context);
    }
    let aliases = self.control_candidates(prefix, context);
    let mut candidates: BTreeMap<_, _> = self
      .named(prefix, context)
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
      if !name.starts_with(prefix) {
        continue;
      }
      if name != prefix && self.dispatch_alias(row) {
        continue;
      }
      if self.names.get(&name).is_some_and(|id| id.as_str() == name) {
        continue;
      }
      let key = KeyTopic::new(row);
      candidates.insert(
        name.clone(),
        Candidate {
          name,
          description: format!("{} in {} mode", row.purpose, context.name()),
          action: Some(key.action.clone()),
          destination: Destination::Key(key),
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
          && matches!(candidate.destination, Destination::Action(_))
          && self.names.get(&candidate.name).is_some_and(TopicId::parameterized)
          && !prefix.contains('.')
        {
          return false;
        }
        let page = matches!(&candidate.destination,
          Destination::Catalog(id) if id.as_str() != ACTIONS)
          || matches!(candidate.destination, Destination::Setting(_));
        !prefix.is_empty()
          || candidate.action.is_some()
          || (self
            .names
            .get(&candidate.name)
            .is_some_and(|id| id.as_str() == candidate.name)
            && !self.names[&candidate.name].parameterized()
            && !page)
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
