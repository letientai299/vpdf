use crate::Result;
use std::sync::{Arc, mpsc::TryRecvError};
use vpdf_model::{document::Target, text::PageId, theme::Theme};
use vpdf_pdf::{
  jobs::Wake,
  manual::{Document, Job},
};
use vpdf_ui::topics::{Reference, TopicId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
  topic: TopicId,
  anchor: Option<String>,
  offset: i32,
  origin: Origin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Origin {
  Anchor,
  Glyph(i32),
  PageBreak,
}

impl Location {
  pub fn topic(topic: TopicId) -> Self {
    Self { topic, anchor: None, offset: 0, origin: Origin::Anchor }
  }

  pub fn anchored(topic: TopicId, anchor: &str) -> Result<Self> {
    anyhow::ensure!(
      !anchor.is_empty()
        && anchor
          .bytes()
          .all(|ch| ch.is_ascii_alphanumeric() || b"._-".contains(&ch)),
      "Invalid help anchor"
    );
    Ok(Self {
      topic,
      anchor: Some(anchor.into()),
      offset: 0,
      origin: Origin::Anchor,
    })
  }

  pub fn at(document: &Document, page: PageId, y: f32) -> Option<Self> {
    let anchor = document.anchor_at(page, y)?;
    Some(Self {
      topic: TopicId::parse(&anchor.topic).ok()?,
      anchor: anchor.name.clone(),
      offset: match anchor.target {
        Target::Point { page: start_page, y: start, .. } => {
          (((page.0 - start_page.0) as f32 * 792.0 + y - start).max(0.0)
            * 1000.0)
            .round() as i32
        }
        _ => 0,
      },
      origin: Origin::Anchor,
    })
  }

  pub fn point(
    document: &Document,
    page: PageId,
    x: f32,
    y: f32,
  ) -> Option<Self> {
    let mut location = Self::at(document, page, y)?;
    location.origin = Origin::Glyph((x * 1000.0).round() as i32);
    Some(location)
  }

  pub fn page_break(document: &Document, page: PageId) -> Option<Self> {
    let mut location = Self::at(document, page, 791.0)?;
    location.origin = Origin::PageBreak;
    Some(location)
  }

  pub fn is_page_break(&self) -> bool {
    matches!(self.origin, Origin::PageBreak)
  }

  pub fn id(&self) -> &TopicId {
    &self.topic
  }

  pub(crate) fn target(&self, document: &Document) -> Option<Target> {
    let mut target = document
      .topics
      .get(&self.name())
      .or_else(|| document.topics.get(self.topic.as_str()))?
      .clone();
    if document.topics.contains_key(&self.name())
      && let Target::Point { page, x, y } = &mut target
    {
      *y += self.offset as f32 / 1000.0;
      page.0 += (*y / 792.0).floor() as usize;
      *y %= 792.0;
      if let Origin::Glyph(saved) = self.origin {
        *x = saved as f32 / 1000.0;
      }
    }
    Some(target)
  }

  fn name(&self) -> String {
    match &self.anchor {
      Some(anchor) => format!("{}.{}", self.topic.as_str(), anchor),
      None => self.topic.as_str().into(),
    }
  }
}

#[derive(PartialEq)]
struct Key {
  manual: String,
  theme: Theme,
}

impl Key {
  fn new(reference: &Reference) -> Self {
    let theme = reference
      .configuration
      .settings
      .iter()
      .find(|setting| setting.name == "appearance.theme")
      .and_then(|setting| setting.effective.as_str())
      .and_then(Theme::parse)
      .unwrap_or_default();
    Self { manual: super::identity(), theme }
  }
}

#[derive(Default)]
enum State {
  #[default]
  Empty,
  Building {
    key: Key,
    job: Job,
  },
  Cached {
    key: Key,
    document: Arc<Document>,
  },
}

impl State {
  fn key(&self) -> Option<&Key> {
    match self {
      Self::Empty => None,
      Self::Building { key, .. } | Self::Cached { key, .. } => Some(key),
    }
  }
}

#[derive(Clone, Copy)]
enum Delivery {
  Open,
  Refresh,
}

pub struct Ready {
  pub document: Arc<Document>,
  pub location: Location,
  pub target: Target,
  pub fallback: Option<Location>,
}

/// One cache per help session.
#[derive(Default)]
pub struct Runtime {
  state: State,
  selected: Option<Location>,
  delivery: Option<Delivery>,
}

impl Runtime {
  pub(crate) fn reuse(
    &mut self,
    reference: &Reference,
    document: Arc<Document>,
  ) {
    self.state = State::Cached { key: Key::new(reference), document };
    self.delivery = None;
  }

  pub fn request(
    &mut self,
    reference: &Reference,
    topic: TopicId,
    wake: &Wake,
  ) {
    self.selected = Some(Location::topic(topic));
    self.start(reference, Delivery::Open, wake);
  }

  pub fn locate(&mut self, location: Location) {
    self.selected = Some(location);
  }

  fn start(&mut self, reference: &Reference, delivery: Delivery, wake: &Wake) {
    let key = Key::new(reference);
    self.delivery = Some(delivery);
    if self.state.key() != Some(&key) {
      let job = vpdf_pdf::manual::spawn(
        super::PDF,
        key.theme.palette(),
        super::reference::chapters,
        wake,
      );
      self.state = State::Building { key, job };
    } else if matches!(self.state, State::Cached { .. }) {
      wake.wake();
    }
  }

  pub fn refresh(&mut self, reference: &Reference, wake: &Wake) {
    if self.selected.is_some() {
      self.start(reference, Delivery::Refresh, wake);
    }
  }

  pub fn poll(&mut self) -> Option<Result<Ready>> {
    let delivery = self.delivery?;
    if let State::Building { job, .. } = &self.state {
      let result = match job.result.try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => return None,
        Err(TryRecvError::Disconnected) => {
          Err(anyhow::anyhow!("Help worker disconnected"))
        }
      };
      let State::Building { key, .. } = std::mem::take(&mut self.state) else {
        unreachable!()
      };
      match result {
        Ok(document) => self.state = State::Cached { key, document },
        Err(error) => {
          self.delivery = None;
          return Some(Err(error));
        }
      }
    }
    let State::Cached { document, .. } = &self.state else { return None };
    let mut location = self.selected.clone()?;
    let mut fallback = None;
    self.delivery = None;
    if matches!(delivery, Delivery::Refresh)
      && !document.topics.contains_key(&location.name())
    {
      fallback = Some(location.clone());
      location.anchor = None;
      location.offset = 0;
      location.origin = Origin::Anchor;
      if !document.topics.contains_key(location.topic.as_str()) {
        location.topic = match TopicId::parse("config") {
          Ok(topic) => topic,
          Err(error) => return Some(Err(error)),
        };
      }
    }
    let target = location.target(document);
    self.selected = Some(location.clone());
    Some(target.ok_or_else(|| anyhow::anyhow!("No help topic")).map(|target| {
      Ready { document: Arc::clone(document), location, target, fallback }
    }))
  }
}

#[cfg(test)]
mod tests;
