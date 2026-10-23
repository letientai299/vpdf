use super::{
  Action, Annotation, Destination, Input, Panel, PanelKind, Position, Target,
};
use crate::keys;

#[derive(Debug, PartialEq)]
pub enum Event {
  Unhandled,
  Stay,
  Close,
  Copy { text: String, register: char },
  Preview { position: Position, annotation: Annotation },
  OpenUri(String),
  Follow { focus: bool },
}

enum Route {
  Navigate,
  Unhandled,
  Ignore,
  Close,
  Copy,
  Preview,
  OpenUri,
  Follow,
  Keep,
}

impl Panel {
  pub fn navigate_available(&self, action: Action) -> bool {
    super::view::Navigation::of(action).is_some()
  }

  pub fn input(&mut self, input: Input, page: usize) -> Event {
    if !self.focused() {
      return Event::Unhandled;
    }
    match self.route(input.action) {
      Route::Unhandled => Event::Unhandled,
      Route::Navigate => {
        self.navigate(input, page);
        Event::Stay
      }
      Route::Ignore => Event::Stay,
      Route::Close => Event::Close,
      Route::Copy => self.yank_text().map_or(Event::Stay, |text| Event::Copy {
        text,
        register: input.register,
      }),
      Route::Preview => match self
        .rows
        .get(self.selected)
        .map(|row| &row.destination)
      {
        Some(Destination::Annotation { position, annotation }) => {
          Event::Preview { position: *position, annotation: annotation.clone() }
        }
        _ => Event::Stay,
      },
      Route::OpenUri => {
        match self.rows.get(self.selected).map(|row| &row.destination) {
          Some(Destination::Link(Target::Uri(uri))) => {
            Event::OpenUri(uri.clone())
          }
          _ => Event::Stay,
        }
      }
      Route::Follow => Event::Follow { focus: true },
      Route::Keep => Event::Follow { focus: false },
    }
  }

  /// Menu shortcut for a panel effect.
  pub fn shortcut(&self, action: Action) -> Option<&'static str> {
    keys::key_for(self.shortcut_action(action)?)
  }

  pub fn shortcut_action(&self, action: Action) -> Option<Action> {
    Some(match self.route(action) {
      Route::Close => Action::Quit,
      Route::Copy => Action::YankLine,
      Route::Preview => Action::Peek,
      Route::OpenUri => Action::OpenUrl,
      Route::Follow => Action::Follow,
      Route::Keep => Action::Swap,
      Route::Navigate | Route::Unhandled | Route::Ignore => return None,
    })
  }

  /// Includes actions delegated to the application.
  pub fn allows(&self, action: Action) -> bool {
    !self.focused()
      || matches!(action, Action::Cancel | Action::LocalFocus)
      || !matches!(self.route(action), Route::Ignore)
  }

  pub fn purpose(&self, action: Action) -> Option<&'static str> {
    match action {
      Action::Motion(vpdf_reader::motion::Motion::Down) => {
        return Some("select next row; keep panel focus");
      }
      Action::Motion(vpdf_reader::motion::Motion::Up) => {
        return Some("select previous row; keep panel focus");
      }
      _ => {}
    }
    Some(match self.route(action) {
      Route::Close => "close panel",
      Route::Copy => "copy panel row",
      Route::Preview => "preview annotation",
      Route::OpenUri => "open row URL",
      Route::Follow => "follow panel row",
      Route::Keep => "follow without focusing content",
      Route::Navigate | Route::Unhandled | Route::Ignore => return None,
    })
  }

  fn route(&self, action: Action) -> Route {
    if super::view::Navigation::of(action).is_some() {
      return Route::Navigate;
    }
    match action {
      Action::Quit | Action::ClosePanel => Route::Close,
      Action::Yank | Action::YankLine => Route::Copy,
      // Keep text actions within the panel.
      Action::Motion(_)
      | Action::Visual(_)
      | Action::Object(..)
      | Action::Reselect
      | Action::WordSearch
      | Action::Definition
      | Action::Cancel => Route::Ignore,
      Action::Peek if self.kind == PanelKind::Annotations => Route::Preview,
      Action::OpenUrl => Route::OpenUri,
      Action::Follow => Route::Follow,
      Action::Swap if !self.is_recent_view() => Route::Keep,
      Action::Help
      | Action::YankPath
      | Action::Command
      | Action::Search(_)
      | Action::Window(_)
      | Action::Tab(_)
      | Action::Panel(_)
      | Action::Overview => Route::Unhandled,
      _ if self.is_recent_view() => Route::Ignore,
      _ => Route::Unhandled,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::keys::{Grammar, Keys};
  use crate::panel::Row;
  use vpdf_reader::motion::Motion;

  fn panel(kind: PanelKind) -> Panel {
    let rows = (0..4)
      .map(|line| {
        Row::new(
          format!("row {line}"),
          String::new(),
          Destination::Position(Position { line, ..Position::default() }),
        )
      })
      .collect();
    if kind == PanelKind::Recent {
      Panel::recent(rows, &[])
    } else {
      Panel::new(kind, rows)
    }
  }

  #[test]
  fn shortcuts_produce_the_same_panel_effect() {
    for kind in [PanelKind::Outline, PanelKind::Annotations, PanelKind::Recent]
    {
      for action in [
        Action::Quit,
        Action::ClosePanel,
        Action::Yank,
        Action::YankLine,
        Action::Peek,
        Action::OpenUrl,
        Action::Follow,
        Action::Swap,
      ] {
        let mut expected = panel(kind);
        let Some(shortcut) = expected.shortcut(action) else { continue };
        let input = Input { action, count: 2, explicit: true, register: 'a' };
        let want = expected.input(input, 24);
        let mut keys = Keys::default();
        let events = keys::parse(&format!("\"a2{shortcut}")).unwrap();
        let inputs: Vec<_> = events
          .into_iter()
          .filter_map(|key| keys.feed(key, Grammar::Panel))
          .collect();
        assert_eq!(inputs.len(), 1, "{shortcut}");
        assert_eq!(inputs[0].count, 2);
        assert_eq!(inputs[0].register, 'a');
        assert_eq!(
          panel(kind).input(inputs[0], 24),
          want,
          "{kind:?} {shortcut}"
        );
      }
    }
    assert_eq!(panel(PanelKind::Recent).shortcut(Action::Swap), None);
    assert_eq!(panel(PanelKind::Outline).shortcut(Action::Peek), None);
    assert_eq!(panel(PanelKind::Annotations).shortcut(Action::Peek), Some("K"));
  }

  #[test]
  fn focus_and_recent_scope_preserve_navigation() {
    for kind in [PanelKind::Outline, PanelKind::Recent] {
      let mut view = panel(kind);
      let down = Input {
        action: Action::Motion(Motion::Down),
        count: 2,
        explicit: true,
        register: 'a',
      };
      view.focus(false);
      assert_eq!(view.input(down, 24), Event::Unhandled);
      assert_eq!(view.selected(), 0);
      view.focus(true);
      assert_eq!(view.input(down, 24), Event::Stay);
      assert_eq!(view.selected(), 2);
      assert_eq!(
        view.input(Input { action: Action::YankLine, ..down }, 24),
        Event::Copy { text: "row 2\n".into(), register: 'a' }
      );
      for action in
        [Action::Motion(Motion::Word), Action::Definition, Action::WordSearch]
      {
        assert_eq!(view.input(Input::plain(action), 24), Event::Stay);
      }
      assert_eq!(
        view.input(Input::plain(Action::Command), 24),
        Event::Unhandled
      );
      assert_eq!(
        view.input(Input::plain(Action::Zoom(1)), 24),
        if kind == PanelKind::Recent { Event::Stay } else { Event::Unhandled }
      );
    }
  }

  #[test]
  fn selected_values_leave_as_effects() {
    let position = Position::default();
    let annotation = Annotation {
      kind: "Note".into(),
      author: "Reader".into(),
      contents: "A note".into(),
      areas: Vec::new(),
    };
    let mut view = Panel::new(
      PanelKind::Annotations,
      vec![Row::new(
        "note".into(),
        String::new(),
        Destination::Annotation { position, annotation: annotation.clone() },
      )],
    );
    assert_eq!(
      view.input(Input::plain(Action::Peek), 24),
      Event::Preview { position, annotation }
    );
    let uri = "https://example.org/paper";
    let mut view = Panel::new(
      PanelKind::Links,
      vec![Row::new(
        "paper".into(),
        String::new(),
        Destination::Link(Target::Uri(uri.into())),
      )],
    );
    assert_eq!(
      view.input(Input::plain(Action::OpenUrl), 24),
      Event::OpenUri(uri.into())
    );
    assert_eq!(
      view.input(Input::plain(Action::YankLine), 24),
      Event::Copy { text: format!("{uri}\n"), register: '"' }
    );
  }
}
