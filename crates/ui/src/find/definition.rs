use super::Search;
use crate::Result;
use crate::keys::{self, Action};
use crate::source::Outline;
use crate::text::Position;
use vpdf_model::document::Target;
use vpdf_reader::reader::{JumpDescription, TextSource};
use vpdf_search::reference::{Lookup, Reference};

#[derive(Debug, PartialEq)]
pub enum Definition {
  Unchanged,
  External,
  NoReference,
  Follow(Target),
  Outline(usize),
  Search(Lookup),
  Found { position: Position, description: JumpDescription },
  Candidates,
  NotFound,
}

impl Definition {
  pub fn resolve(
    position: Position,
    target: Option<Target>,
    source: &mut (impl TextSource + Outline),
  ) -> Result<Self> {
    if let Some(target) = target {
      return Ok(match target {
        Target::Uri(_) => Self::External,
        _ => Self::Follow(target),
      });
    }
    let text = source.page_text(position.page)?;
    let Some(line) = text.lines.get(position.line) else {
      return Ok(Self::Unchanged);
    };
    let Some(reference) = Reference::at_glyph(line, position.glyph) else {
      return Ok(Self::NoReference);
    };
    let targets = reference.headings(source.outline(), source.page_count());
    Ok(match targets.as_slice() {
      [] => Self::Search(Lookup { reference, origin: position }),
      [(_, target)] => Self::Follow((*target).clone()),
      [(first, _), ..] => Self::Outline(*first),
    })
  }

  pub fn message(&self) -> Option<String> {
    let message = match self {
      Self::Unchanged | Self::Follow(_) => return None,
      Self::External => {
        return Some(keys::key_for(Action::OpenUrl).map_or_else(
          || "External link".to_owned(),
          |key| format!("External link: use {}", keys::notation(key)),
        ));
      }
      Self::NoReference => "No document reference at target",
      Self::Outline(_) => {
        "Multiple heading candidates; choose an outline entry"
      }
      Self::Search(_) => "Finding reference…",
      Self::Found { .. } => "Reference found",
      Self::Candidates => "Multiple reference candidates; choose a result",
      Self::NotFound => "Reference not found",
    };
    Some(message.to_owned())
  }
}

impl Search {
  pub fn definition_result(&self) -> Definition {
    match self.hits() {
      [] => Definition::NotFound,
      [hit] => Definition::Found {
        position: hit.first,
        description: self.description(hit.first),
      },
      _ => Definition::Candidates,
    }
  }
}
