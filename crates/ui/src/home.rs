#[cfg(test)]
mod recent_tests;
pub(crate) mod welcome;

pub use welcome::Picker;

use crate::Result;
use crate::bindings::{Bindings, Context};
use crate::keys::{Action, Dispatch, Keys};
use crate::layout::Screen;
use crate::panel::Panel;
use crate::panel::recent::{Entry, Shelf};
use crate::panel::{Destination, Row};
use crate::prompt::{History, Prompt, PromptEvent, PromptKind};
use crate::theme::Color;
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;
use vpdf_model::geometry::At;
use vpdf_model::input::Event;
use vpdf_model::input::{KeyEvent, KeyEventKind};
use vpdf_model::scene::{Draw, Op, Surface, Text};
use vpdf_model::style::Role;

#[derive(Debug, Eq, PartialEq)]
pub enum Effect {
  Stay,
  Open(PathBuf),
  Forget(Vec<PathBuf>),
  Quit,
  Command(String),
  Paste(String),
  Editor(String),
  Complete(String),
}

/// Start screen: recent documents and a command line to open files.
pub struct Home {
  screen: Screen,
  at: At,
  error: Option<String>,
  panel: Panel,
  info: Option<Panel>,
  prompt: Option<Prompt>,
  keys: Keys,
  bindings: Bindings,
  picker: Picker,
}

impl Home {
  pub fn new(
    rows: Vec<Row>,
    recent: &[Entry],
    screen: Screen,
    picker: Picker,
  ) -> Self {
    let panel = Panel::recent(rows, recent);
    Self {
      screen,
      at: At::default(),
      error: None,
      panel,
      info: None,
      prompt: None,
      keys: Keys::default(),
      bindings: Bindings::default(),
      picker,
    }
  }

  pub fn refresh_recent(&mut self, rows: Vec<Row>, entries: &[Entry]) {
    self.panel.refresh_recent(rows, entries);
  }

  pub fn set_bindings(&mut self, bindings: Bindings) {
    self.bindings = bindings;
    if let Some(prompt) = &mut self.prompt {
      prompt.set_bindings(self.bindings.map(Context::prompt(prompt.kind())));
    }
  }

  pub fn reset_input(&mut self) {
    self.keys.clear();
    if let Some(prompt) = &mut self.prompt {
      prompt.reset_input();
    }
  }

  pub fn clear_error(&mut self) {
    self.error = None;
  }

  pub fn invocation(&self) -> crate::topics::Invocation {
    let context = if self.info.is_none() && !self.panel.rows().is_empty() {
      Context::HomeRecent
    } else {
      Context::Home
    };
    crate::topics::Invocation::home().in_context(context)
  }

  pub fn screen(&self) -> Screen {
    self.screen
  }

  pub fn resize(&mut self, screen: Screen, at: At) {
    self.screen = screen;
    self.at = at;
  }

  pub fn error(&self) -> Option<&str> {
    self.error.as_deref()
  }

  pub fn fail(&mut self, error: String) {
    self.error = Some(error);
  }

  pub fn show_info(&mut self, kind: crate::keys::PanelKind, text: &str) {
    self.panel.leave_removal();
    let mut info = Panel::grouped(kind, Row::sections(text), self.screen.rows);
    // Home has no fold keys; every group shows.
    info.fold(crate::keys::Fold::ExpandAll, 1);
    self.info = Some(info);
  }

  pub fn shelf_mut(&mut self) -> Option<&mut Shelf> {
    self.panel.shelf_mut()
  }

  pub fn scene<'a, S: Surface<'a>>(
    &mut self,
    scene: &mut S,
    now: u64,
    request: impl FnOnce(&mut Shelf),
  ) -> std::result::Result<(), S::Error> {
    if self.screen.columns == 0 || self.screen.rows == 0 {
      return Ok(());
    }
    self.scene_clear(scene);
    if self.info.is_none() && self.panel.rows().is_empty() {
      welcome::scene(scene, self.screen, self.at, self.picker);
      self.panel.scene_confirmation(
        scene,
        self.at,
        self.screen.columns,
        self.screen.rows,
      );
    } else {
      let drawn = self.info.as_mut().unwrap_or(&mut self.panel).scene(
        scene,
        crate::panel::recent::Frame {
          at: self.at,
          width: self.screen.columns,
          screen: self.screen,
          now,
        },
        true,
      );
      if let Some(cards) = self.panel.shelf_mut() {
        request(cards);
      }
      drawn?;
    }
    if let Some(prompt) = &self.prompt {
      prompt.scene(scene, self.screen, self.at, None);
    } else if self.screen.rows > 1 || !self.confirming() {
      self.scene_footer(scene);
    }
    Ok(())
  }

  pub fn scene_clear<'a>(&self, scene: &mut impl Draw<'a>) {
    crate::paint::clear(scene, self.at, self.screen);
  }

  fn footer(&self) -> String {
    let context =
      if self.info.is_some() { Context::Home } else { Context::HomeRecent };
    if self.bindings == Bindings::default() {
      if self.info.is_some() {
        "j/k scroll  Esc close".into()
      } else if self.panel.rows().is_empty() {
        "Enter open  :help  :e file  :q quit".into()
      } else {
        "Enter open  Space select  x remove  :help  :q".into()
      }
    } else {
      let mut hints = if self.info.is_some() {
        vec![
          self.bindings.hint(
            context,
            &crate::bindings::Target::Reader(Action::Motion(
              vpdf_reader::motion::Motion::Down,
            )),
            "down",
          ),
          self.bindings.hint(
            context,
            &crate::bindings::Target::Reader(Action::Motion(
              vpdf_reader::motion::Motion::Up,
            )),
            "up",
          ),
          self.bindings.hint(
            context,
            &crate::bindings::Target::Reader(Action::Cancel),
            "close",
          ),
        ]
      } else {
        vec![
          self.bindings.hint(
            context,
            &crate::bindings::Target::Reader(Action::Follow),
            "open",
          ),
          self.bindings.hint(
            context,
            &crate::bindings::Target::Reader(Action::Command),
            "command",
          ),
          self.bindings.hint(
            context,
            &crate::bindings::Target::Reader(Action::Help),
            "help",
          ),
          self.bindings.hint(
            context,
            &crate::bindings::Target::Reader(Action::Quit),
            "quit",
          ),
        ]
      };
      if self.info.is_none() && !self.panel.rows().is_empty() {
        for (action, label) in
          [(Action::RecentToggle, "select"), (Action::RecentRemove, "remove")]
        {
          hints.push(self.bindings.hint(
            context,
            &crate::bindings::Target::Reader(action),
            label,
          ));
        }
      }
      hints
        .into_iter()
        .filter(|hint| !hint.is_empty())
        .collect::<Vec<_>>()
        .join("  ")
    }
  }

  pub fn scene_footer<'a>(&self, scene: &mut impl Draw<'a>) {
    let footer = self.footer();
    let (text, style) = match &self.error {
      Some(error) => (error.as_str(), Role::Error),
      None => (footer.as_str(), Role::Bar),
    };
    let columns = usize::from(self.screen.columns);
    if columns < 8 {
      scene.push(Op::Style(style));
      crate::paint::line(
        scene,
        self.at,
        self.screen.rows.saturating_sub(1),
        text,
        self.screen.columns,
      );
      scene.extend([Op::Style(Role::Reset), Op::HideCursor]);
      return;
    }
    let text = vpdf_model::text::clean(text, columns.saturating_sub(8));
    let fill = columns.saturating_sub(7 + text.width());
    scene.extend([
      Op::Move {
        column: usize::from(self.at.column),
        row: usize::from(self.at.row)
          + usize::from(self.screen.rows.saturating_sub(1)),
      },
      Op::Style(Role::Badge(Color::Blue)),
      Op::Text(Text::new(" Vpdf ")),
      Op::Style(style),
      Op::Text(Text::new(" ")),
      Op::Text(Text::new(text)),
      Op::Spaces(fill),
      Op::Style(Role::Reset),
      Op::HideCursor,
    ]);
  }

  pub fn confirming(&self) -> bool {
    self.panel.confirming()
  }

  /// Whether a prompt takes typed and pasted text.
  pub fn typing(&self) -> bool {
    self.prompt.is_some()
  }

  pub fn replace_prompt(&mut self, text: &str) {
    if let Some(prompt) = &mut self.prompt {
      prompt.replace(text);
      self.preview_prompt();
    }
  }

  pub fn complete_prompt(&mut self, start: usize, items: Vec<String>) {
    if let Some(prompt) = &mut self.prompt {
      prompt.complete(start, items);
      self.preview_prompt();
    }
  }

  pub fn describe_prompt(&mut self, label: impl Fn(&str) -> String) {
    if let Some(prompt) = &mut self.prompt {
      prompt.describe(label);
    }
  }

  pub fn open_prompt(&mut self, prompt: Prompt) {
    self.prompt = Some(prompt);
  }

  fn preview_prompt(&mut self) {
    if let Some(prompt) = &self.prompt
      && matches!(prompt.kind(), PromptKind::Search(_))
    {
      self.panel.preview(prompt.text());
    }
  }

  /// A report in the view's cells, above its bottom row.
  fn local_mouse(
    &self,
    mouse: vpdf_model::input::Mouse,
  ) -> Option<vpdf_model::input::MouseEvent> {
    let (rows, cell) = (self.screen.rows.saturating_sub(1), self.screen.cell());
    mouse.cells(cell).and_then(|mut mouse| {
      mouse.row = mouse.row.checked_sub(self.at.row)?;
      mouse.column = mouse.column.checked_sub(self.at.column)?;
      (mouse.row < rows).then_some(mouse)
    })
  }

  pub fn event(
    &mut self,
    event: Event,
    history: &mut History,
  ) -> Result<Effect> {
    if self.panel.confirming() {
      return Ok(match event {
        Event::Key(key) => match self.panel.confirm_key(key) {
          crate::panel::Event::Forget(paths) => Effect::Forget(paths),
          _ => Effect::Stay,
        },
        Event::Mouse(mouse) => match self.local_mouse(mouse) {
          Some(mouse) => match self.panel.confirm_press(mouse) {
            crate::panel::Event::Forget(paths) => Effect::Forget(paths),
            _ => Effect::Stay,
          },
          None => Effect::Stay,
        },
        Event::Paste(_) => Effect::Stay,
      });
    }
    match event {
      Event::Key(key) if key.kind != KeyEventKind::Release => {
        self.error = None;
        if let Some(info) = &mut self.info {
          match self.keys.feed_with(key, Context::Home, &self.bindings) {
            Some(Dispatch::Command(command)) => {
              return Ok(Effect::Command(command));
            }
            Some(Dispatch::Input(input))
              if info.navigate(input, usize::from(self.screen.rows)) =>
            {
              return Ok(Effect::Stay);
            }

            None if self.keys.awaiting().is_some() => return Ok(Effect::Stay),
            None
              if crate::bindings::Sequence::from_keys(&[key]).is_ok_and(
                |sequence| {
                  self.bindings.map(Context::Home).is_unbound(&sequence)
                },
              ) =>
            {
              return Ok(Effect::Stay);
            }
            _ => {}
          }
          self.info = None;
          self.keys = Keys::default();
          return Ok(Effect::Stay);
        }
        self.key(key, history)
      }
      Event::Paste(text) => {
        // Terminals without drops paste dropped files' paths.
        let Some(prompt) = &mut self.prompt else {
          return Ok(Effect::Paste(text));
        };
        let event = prompt.paste(&text);
        self.prompt_event(event, history)
      }
      // An open prompt owns the input.
      Event::Mouse(mouse) if self.prompt.is_none() => {
        let Some(mouse) = self.local_mouse(mouse) else {
          return Ok(Effect::Stay);
        };
        if let Some(info) = &mut self.info {
          info.pointer(mouse);
          return Ok(Effect::Stay);
        }
        Ok(match self.panel.pointer(mouse) {
          Some(_) => self.selected(),
          None => Effect::Stay,
        })
      }
      _ => Ok(Effect::Stay),
    }
  }

  fn key(&mut self, key: KeyEvent, history: &mut History) -> Result<Effect> {
    let Some(prompt) = &mut self.prompt else {
      let Some(dispatch) =
        self.keys.feed_with(key, Context::HomeRecent, &self.bindings)
      else {
        return Ok(Effect::Stay);
      };
      let input = match dispatch {
        Dispatch::Command(command) => return Ok(Effect::Command(command)),
        Dispatch::Input(input) => input,
      };
      if matches!(
        input.action,
        Action::RecentToggle | Action::RecentRemove | Action::Cancel
      ) {
        self.panel.input(input, usize::from(self.screen.rows));
        return Ok(Effect::Stay);
      }
      if self.panel.navigate(input, usize::from(self.screen.rows)) {
        return Ok(Effect::Stay);
      }
      return Ok(match input.action {
        Action::Follow | Action::Swap => self.selected(),
        Action::Command => {
          self.prompt = Some(
            Prompt::new(PromptKind::Command).with_invocation(self.invocation()),
          );
          Effect::Stay
        }
        Action::Search(backward) => {
          self.panel.begin_search(backward);
          self.prompt = Some(Prompt::new(PromptKind::Search(backward)));
          Effect::Stay
        }
        Action::Help => Effect::Command("help".into()),
        Action::Quit => Effect::Quit,
        _ => Effect::Stay,
      });
    };
    prompt.set_bindings(self.bindings.map(Context::prompt(prompt.kind())));
    let event = prompt.feed(key, history);
    self.prompt_event(event, history)
  }

  fn prompt_event(
    &mut self,
    event: PromptEvent,
    history: &mut History,
  ) -> Result<Effect> {
    let Some(prompt) = &mut self.prompt else { return Ok(Effect::Stay) };
    let kind = prompt.kind();
    let search = matches!(kind, PromptKind::Search(_));
    match event {
      PromptEvent::Edited => self.preview_prompt(),
      PromptEvent::Editor => {
        return Ok(Effect::Editor(prompt.text().to_owned()));
      }
      PromptEvent::Complete if kind == PromptKind::Command => {
        return Ok(Effect::Complete(prompt.before_cursor().to_owned()));
      }
      PromptEvent::Cancel => {
        self.prompt = None;
        self.panel.abort_search();
      }
      PromptEvent::Command => {
        return Ok(prompt.take_command().map_or(Effect::Stay, Effect::Command));
      }
      PromptEvent::Submit => {
        let text = prompt.text().to_owned();
        self.prompt = None;
        history.push(kind, &text);
        if !search {
          return Ok(Effect::Command(text));
        }
        anyhow::ensure!(
          self.panel.finish_search(&text),
          "Pattern not found: {}",
          self.panel.query()
        );
      }
      _ => {}
    }
    Ok(Effect::Stay)
  }

  fn selected(&self) -> Effect {
    match self.panel.rows().get(self.panel.selected()).map(|r| &r.destination) {
      Some(Destination::File(path)) => Effect::Open(path.clone()),
      _ => Effect::Stay,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn version_scrolls_on_small_screens() -> Result<()> {
    use vpdf_model::input::{KeyCode, KeyModifiers};
    let screen = crate::testing::screen::cells(16, 6);
    let mut home =
      Home::new(Vec::new(), &[], screen, Picker::Terminal { fzf: true });
    home.show_info(
      crate::keys::PanelKind::Version,
      concat!(
        "A description that wraps across several rows in a narrow window\n",
        "Repository address\nvpdf build"
      ),
    );
    let mut before = crate::testing::empty();
    home.scene(&mut before, 0, |_| {})?;
    assert!(!crate::testing::text(&before.into_scene()).contains("vpdf build"));
    let mut history = History::default();
    home.event(
      Event::Key(KeyEvent::new(KeyCode::Char('G'), KeyModifiers::NONE)),
      &mut history,
    )?;
    let mut after = crate::testing::empty();
    home.scene(&mut after, 0, |_| {})?;
    let text = crate::testing::text(&after.into_scene());
    assert!(text.contains("VERSION"), "{text}");
    assert!(text.contains("vpdf build"), "{text}");
    home.event(
      Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
      &mut history,
    )?;
    assert!(home.info.is_none());
    Ok(())
  }

  #[test]
  fn effects_resume_the_prompt() -> Result<()> {
    use vpdf_model::input::{KeyCode, KeyModifiers};
    let screen = crate::testing::screen::cells(80, 24);
    let rows = ["paper.pdf", "book.pdf"]
      .map(|path| {
        Row::new(path.into(), String::new(), Destination::File(path.into()))
      })
      .into();
    let mut home = Home::new(rows, &[], screen, Picker::Terminal { fzf: true });
    let mut history = History::default();
    let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    home.event(key(KeyCode::Char(':')), &mut history)?;
    assert_eq!(
      home.prompt.as_ref().unwrap().invocation(),
      Some(&crate::topics::Invocation::Context(Context::HomeRecent))
    );
    home.event(Event::Paste("e ".into()), &mut history)?;
    assert_eq!(
      home.event(key(KeyCode::Tab), &mut history)?,
      Effect::Complete("e ".into())
    );
    home.complete_prompt(2, vec!["paper.pdf".into()]);
    assert_eq!(
      home.event(key(KeyCode::Enter), &mut history)?,
      Effect::Command("e paper.pdf".into())
    );
    assert!(!home.typing());
    home.event(key(KeyCode::Char('/')), &mut history)?;
    home.event(
      Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
      &mut history,
    )?;
    assert_eq!(
      home.event(key(KeyCode::Char('e')), &mut history)?,
      Effect::Editor(String::new())
    );
    home.replace_prompt("book");
    assert_eq!(home.panel.selected(), 1);
    assert_eq!(home.event(key(KeyCode::Enter), &mut history)?, Effect::Stay);
    assert_eq!(
      home.event(key(KeyCode::Enter), &mut history)?,
      Effect::Open("book.pdf".into())
    );
    assert_eq!(
      home.event(Event::Paste("paper.pdf".into()), &mut history)?,
      Effect::Paste("paper.pdf".into())
    );
    Ok(())
  }

  #[test]
  fn panel_ends_at_the_last_column() -> Result<()> {
    let screen = crate::testing::screen::cells(40, 10);
    let mut recorder = crate::testing::empty();
    Home::new(
      vec![Row::new(
        "paper.pdf".into(),
        String::new(),
        Destination::File("paper.pdf".into()),
      )],
      &[],
      screen,
      Picker::Terminal { fzf: true },
    )
    .scene(&mut recorder, 0, |_| {})?;
    let scene = recorder.into_scene();
    assert!(crate::testing::text(&scene).contains(" Vpdf "));
    let columns = scene.iter().filter_map(|op| match op {
      Op::Move { column, .. } => Some(*column),
      _ => None,
    });
    assert_eq!(columns.max(), Some(usize::from(screen.columns - 1)));
    for row in 0..screen.rows - 1 {
      assert!(scene.windows(3).any(|ops| ops[0]
        == Op::Move {
          column: usize::from(screen.columns - 1),
          row: usize::from(row)
        }
        && ops[1] == Op::Style(Role::Muted)
        && matches!(&ops[2], Op::Text(text) if text.as_str() == "│")));
    }
    Ok(())
  }

  #[test]
  fn top_stats_offset_home_and_mouse() -> Result<()> {
    use vpdf_model::input::KeyModifiers;
    use vpdf_model::input::{Button, Mouse, MouseKind, Spot};
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let rows = || {
      vec![Row::new(file.into(), String::new(), Destination::File(file.into()))]
    };
    let screen =
      Screen { origin: (0, 4), ..crate::testing::screen::cells(80, 20) };
    let mut home =
      Home::new(rows(), &[], screen, Picker::Terminal { fzf: true });
    home.at.row = 4;
    let mut recorder = crate::testing::empty();
    home.scene(&mut recorder, 0, |_| {})?;
    let scene = recorder.into_scene();
    assert!(scene.contains(&Op::Move { row: 4, column: 0 }));
    assert!(!scene.contains(&Op::Move { row: 0, column: 0 }));
    for (row, expected) in [(1, Effect::Stay), (5, Effect::Open(file.into()))] {
      let click = Event::Mouse(Mouse {
        kind: MouseKind::Press(Button::LEFT),
        at: Spot::Cell { column: 5, row },
        modifiers: KeyModifiers::NONE,
      });
      assert_eq!(home.event(click, &mut History::default())?, expected);
    }
    Ok(())
  }

  #[test]
  fn clicks_wait_for_an_open_prompt() -> Result<()> {
    use vpdf_model::input::{Button, Mouse, MouseKind, Spot};
    use vpdf_model::input::{KeyCode, KeyModifiers};
    let file = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let rows = || {
      vec![Row::new(file.into(), String::new(), Destination::File(file.into()))]
    };
    let screen = crate::testing::screen::cells(80, 24);
    let click = Event::Mouse(Mouse {
      kind: MouseKind::Press(Button::LEFT),
      at: Spot::Cell { column: 5, row: 1 },
      modifiers: KeyModifiers::NONE,
    });
    for (typed, expected) in [
      (":", Effect::Stay),
      ("/", Effect::Stay),
      ("", Effect::Open(file.into())),
    ] {
      let mut home =
        Home::new(rows(), &[], screen, Picker::Terminal { fzf: true });
      let mut history = History::default();
      for ch in typed.chars() {
        let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
        home.event(Event::Key(key), &mut history)?;
      }
      assert_eq!(home.event(click.clone(), &mut history)?, expected, "{typed}");
    }
    Ok(())
  }
}

#[cfg(test)]
#[path = "home/tests.rs"]
mod welcome_tests;
