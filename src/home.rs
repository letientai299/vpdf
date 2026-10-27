use crate::Result;
use crate::cmdline;
use crate::recent::Loader;
use crate::state::Entry;
use std::io::Write;
use std::path::PathBuf;
use vpdf_model::input::Event;
use vpdf_model::style::Role;
use vpdf_terminal::graphics::Graphics;
use vpdf_terminal::style::sgr;
use vpdf_terminal::terminal;
use vpdf_ui::commands::View;
use vpdf_ui::home::Effect;
use vpdf_ui::layout::Screen;
use vpdf_ui::prompt::History;

#[derive(Debug, PartialEq)]
pub enum Outcome {
  Stay,
  Command(String),
  Input(vpdf_ui::keys::Input),
  Dismiss,
  Open(PathBuf),
  Reference(vpdf_model::reference::Reference),
  Forget(Vec<PathBuf>),
  Quit,
}

pub struct Home {
  pub view: vpdf_ui::home::Home,
  loader: Loader,
  graphics: Graphics,
  settings: crate::config::Settings,
  focused: bool,
}

impl Home {
  pub fn new(
    recent: &[Entry],
    screen: Screen,
    wake: &vpdf_pdf::jobs::Wake,
    terminal: bool,
  ) -> Self {
    let mut view = vpdf_ui::home::Home::new(
      crate::panel::recent_rows(recent),
      &crate::recent::entries(recent),
      screen,
      if terminal {
        vpdf_ui::home::Picker::Terminal { fzf: cmdline::on_path("fzf") }
      } else {
        vpdf_ui::home::Picker::Native
      },
    );
    view.set_pictures(
      !terminal
        || vpdf_terminal::caps::get().is_none_or(|caps| {
          !matches!(
            caps.profile(),
            vpdf_terminal::caps::Profile::Unsupported(_)
          )
        }),
    );
    Self {
      view,
      loader: Loader::new("", wake),
      graphics: Graphics::live(),
      settings: crate::config::Settings::default(),
      focused: true,
    }
  }

  pub(crate) fn with_layout(
    mut self,
    preferences: vpdf_ui::panel::recent::Preferences,
  ) -> Self {
    self.view.shelf_mut().unwrap().set_preferences(preferences);
    self
  }

  pub(crate) fn configure(&mut self, settings: &crate::config::Settings) {
    self.settings = settings.clone();
    self.view.set_bindings(settings.bindings().clone());
  }

  pub(crate) fn set_tabline(
    &mut self,
    tabline: crate::app::Tabline,
    focused: bool,
  ) {
    self.view.set_tabs(tabline.names, tabline.active, focused);
    self.focused = focused;
  }

  pub(crate) fn updates(&mut self) -> bool {
    self.view.shelf_mut().is_some_and(|cards| self.loader.updates(cards))
  }

  pub(crate) fn scene<'a, S: vpdf_model::scene::Surface<'a>>(
    &mut self,
    scene: &mut S,
  ) -> std::result::Result<(), S::Error> {
    Self::scene_view(&mut self.view, &mut self.loader, &self.settings, scene)?;
    if !self.focused {
      crate::presentation::inactive(
        scene,
        self.view.screen(),
        0,
        self.settings.opacity(),
      );
    }
    Ok(())
  }

  fn scene_view<'a, S: vpdf_model::scene::Surface<'a>>(
    view: &mut vpdf_ui::home::Home,
    loader: &mut Loader,
    settings: &crate::config::Settings,
    scene: &mut S,
  ) -> std::result::Result<(), S::Error> {
    settings.scene_theme(scene, vpdf_model::theme::Colors::Terminal);
    view.scene(scene, crate::state::now(), |cards| loader.request(cards))
  }

  pub fn draw(&mut self, out: &mut impl Write) -> Result<()> {
    write!(out, "{}{}", terminal::frame_start(), sgr(Role::Reset))?;
    self.updates();
    self.graphics.begin(out)?;
    crate::presentation::draw(out, Some(&mut self.graphics), |scene| {
      Self::scene_view(&mut self.view, &mut self.loader, &self.settings, scene)
    })??;
    self.graphics.end(out)?;
    write!(out, "{}", terminal::frame_end())?;
    out.flush()?;
    Ok(())
  }

  pub(crate) fn images_lost(&mut self) {
    self.graphics.lost();
  }

  pub fn release(&mut self, out: &mut impl Write) -> Result<()> {
    self.graphics.clear(out)
  }

  pub fn event(
    &mut self,
    event: vpdf_terminal::decode::Event,
    history: &mut History,
  ) -> Result<Outcome> {
    let Some(event) = crate::input::event(event) else {
      return Ok(Outcome::Stay);
    };
    self.handle(event, history, true)
  }

  pub(crate) fn handle(
    &mut self,
    event: Event,
    history: &mut History,
    terminal: bool,
  ) -> Result<Outcome> {
    Ok(match self.view.event(event, history)? {
      Effect::Stay => Outcome::Stay,
      Effect::Open(path) => Outcome::Open(path),
      Effect::OpenUri(uri) => {
        crate::commands::open_uri(&uri)?;
        Outcome::Stay
      }
      Effect::Forget(paths) => Outcome::Forget(paths),
      Effect::Quit => Outcome::Quit,
      Effect::Command(line) => Outcome::Command(line),
      Effect::Input(input) => Outcome::Input(input),
      Effect::Dismiss => Outcome::Dismiss,
      Effect::Paste(text) => {
        crate::dnd::pdf(&text)?.map_or(Outcome::Stay, Outcome::Reference)
      }
      Effect::Editor(text) => {
        let edited = crate::programs::edit(&text, terminal)?;
        self.view.replace_prompt(&edited);
        Outcome::Stay
      }
      Effect::Complete(before) => {
        let topics = crate::manual::completions(
          &self.settings,
          &self.view.invocation(),
          cmdline::help_query(&before).unwrap_or(""),
        )?;
        let names: Vec<_> =
          topics.iter().map(|item| item.name.clone()).collect();
        let (start, items) =
          cmdline::complete(&before, View::Home, terminal, &names, &[]);
        self.view.complete_prompt(start, items);
        if cmdline::help_query(&before).is_some() {
          self.view.describe_prompt(|name| cmdline::describe(name, &topics));
        }
        Outcome::Stay
      }
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_pdf::jobs::Wake;

  #[test]
  fn missing_history_shows_welcome() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("missing.pdf");
    let entry = crate::state::tests::entry(&path.to_string_lossy(), 0, 0);
    for terminal in [true, false] {
      let mut home = Home::new(
        std::slice::from_ref(&entry),
        crate::screen::cells(80, 24),
        &Wake::none(),
        terminal,
      );
      let mut scene = vpdf_model::scene::Recorder::new(|_| false);
      home.scene(&mut scene)?;
      let text = scene
        .into_scene()
        .iter()
        .filter_map(|op| match op {
          vpdf_model::scene::Op::Text(text) => Some(text.as_str()),
          _ => None,
        })
        .collect::<String>();
      assert!(text.contains(":edit <file>"), "{text}");
      assert!(!text.contains("missing.pdf"), "{text}");
      assert!(!text.contains('│'), "{text}");
      if !terminal {
        assert!(text.contains(":open"), "{text}");
        assert!(text.contains("Open a PDF"), "{text}");
        assert!(!text.contains(":fzf"), "{text}");
      }
    }
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn home_loads_recent_card() -> Result<()> {
    use std::time::{Duration, Instant};
    let path = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
    let entry = crate::state::tests::entry(&path.to_string_lossy(), 0, 0);
    let screen = crate::screen::cells(120, 40);
    let mut home = Home::new(&[entry], screen, &Wake::none(), true);
    home.graphics = Graphics::default();
    let mut output = Vec::new();
    home.draw(&mut output)?;
    assert!(String::from_utf8_lossy(&output).contains("Loading…"));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
      home.updates();
      output.clear();
      home.draw(&mut output)?;
      if String::from_utf8_lossy(&output).contains("Size") {
        break;
      }
      anyhow::ensure!(Instant::now() < deadline, "Home card never loaded");
      std::thread::sleep(Duration::from_millis(5));
    }
    let text = String::from_utf8_lossy(&output);
    for expected in ["Page", "Size", "a=t,"] {
      assert!(text.contains(expected), "{expected}");
    }
    output.clear();
    home.draw(&mut output)?;
    assert!(!output.windows(4).any(|bytes| bytes == b"a=t,"));
    Ok(())
  }

  #[test]
  fn start_completion_uses_home_scope() -> Result<()> {
    use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};
    use vpdf_terminal::decode::Event;
    let screen = crate::screen::cells(80, 24);
    let mut home = Home::new(&[], screen, &Wake::none(), true);
    let mut history = History::default();
    let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    home.event(key(KeyCode::Char(':')), &mut history)?;
    home.event(Event::Paste("re".into()), &mut history)?;
    home.event(key(KeyCode::Tab), &mut history)?;
    assert_eq!(
      home.event(key(KeyCode::Enter), &mut history)?,
      Outcome::Command("recent".into())
    );
    assert_eq!(history.commands().last().map(String::as_str), Some("recent"));
    Ok(())
  }
}
