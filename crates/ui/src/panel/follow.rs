use super::{
  Annotation, Destination, JumpDescription, MouseButton, MouseEvent,
  MouseEventKind, Panel, PanelKind, PathBuf, Position, Target,
};
use crate::find::Search;
use crate::reading::Pane;

#[derive(Debug, PartialEq)]
pub enum Follow {
  File(PathBuf),
  Mark(char),
  Land { position: Position, description: Option<JumpDescription> },
  Link { target: Target, description: Option<JumpDescription> },
  Annotation { position: Position, annotation: Annotation },
  Focus(Focus),
}

#[derive(Debug, PartialEq)]
pub enum Focus {
  Content,
  Annotation { position: Position, annotation: Annotation },
}

impl Focus {
  pub fn apply(self, reader: &mut Pane) -> Option<(Position, Annotation)> {
    reader.cancel();
    match self {
      Self::Content => {
        reader.normal();
        None
      }
      Self::Annotation { position, annotation } => {
        reader.enter_cursor();
        Some((position, annotation))
      }
    }
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Activation {
  Stay,
  Context,
  Split(usize),
  Follow { focus: bool },
  Refocus,
  Control(super::Control),
}

/// Carries a press across pane relayout.
pub struct Press {
  mouse: MouseEvent,
  clicked: Option<usize>,
  covered: bool,
  control: Option<super::Control>,
}

impl Panel {
  pub fn press(&mut self, mouse: MouseEvent) -> Press {
    let covered = self.covers_page();
    let control = self.control_at(mouse.column, mouse.row);
    let clicked = self.pointer(mouse);
    Press { mouse, clicked, covered, control }
  }

  pub fn follow(&self, search: &Search) -> Option<Follow> {
    let row = self.rows.get(self.selected)?;
    Some(match row.destination.clone() {
      Destination::None => return None,
      Destination::File(path) => Follow::File(path),
      Destination::Mark(name) => Follow::Mark(name),
      Destination::Position(position) => Follow::Land {
        position,
        description: (self.kind == PanelKind::Results)
          .then(|| search.description(position)),
      },
      Destination::Jump { position, description } => {
        Follow::Land { position, description }
      }
      Destination::Link(target) => {
        let title = row.label.trim();
        let description = (self.kind == PanelKind::Outline
          && !matches!(target, Target::Uri(_))
          && !title.is_empty())
        .then(|| JumpDescription::Heading(title.to_owned()));
        Follow::Link { target, description }
      }
      Destination::Annotation { position, annotation } => {
        Follow::Annotation { position, annotation }
      }
    })
  }

  pub fn refocus(&self) -> Option<Follow> {
    let row = self.rows.get(self.selected)?;
    Some(match row.destination.clone() {
      Destination::None => return None,
      Destination::Mark(name) => Follow::Mark(name),
      Destination::Annotation { position, annotation } => {
        Follow::Focus(Focus::Annotation { position, annotation })
      }
      _ => Follow::Focus(Focus::Content),
    })
  }
}

impl Press {
  pub fn activate(self, panel: &mut Panel, clicks: u8) -> Activation {
    let Self { mouse, clicked, covered, control } = self;
    if mouse.kind == MouseEventKind::Down(MouseButton::Left)
      && let Some(control) = control
    {
      panel.clicked = None;
      if let super::Control::Branch(source) = control {
        panel.choose(source);
      }
      return Activation::Control(control);
    }
    if mouse.kind == MouseEventKind::Down(MouseButton::Right) {
      return Activation::Context;
    }
    if mouse.kind == MouseEventKind::Down(MouseButton::Middle) {
      return panel
        .row_at(mouse.column, mouse.row)
        .map_or(Activation::Stay, Activation::Split);
    }
    let followed = if matches!(mouse.kind, MouseEventKind::Down(_)) {
      std::mem::replace(&mut panel.clicked, clicked)
    } else {
      panel.clicked
    };
    let Some(index) = clicked else { return Activation::Stay };
    if covered {
      return Activation::Follow { focus: true };
    }
    if clicks > 1 && followed == Some(index) {
      return Activation::Refocus;
    }
    Activation::Follow { focus: false }
  }
}
