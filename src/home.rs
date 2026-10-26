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
use vpdf_ui::commands::{self, Id, View};
use vpdf_ui::home::Effect;
use vpdf_ui::layout::Screen;
use vpdf_ui::prompt::History;

#[derive(Debug, Eq, PartialEq)]
pub enum Outcome {
  Stay,
  ConfigReload,
  Theme(vpdf_model::theme::Theme),
  Perf(Option<bool>),
  Open(PathBuf),
  Forget(Vec<PathBuf>),
  Help(String),
  Pick { tab: bool },
  Quit,
}

pub struct Home {
  pub view: vpdf_ui::home::Home,
  loader: Loader,
  graphics: Graphics,
  theme: vpdf_model::theme::Theme,
  settings: crate::config::Settings,
}

impl Home {
  pub fn new(
    recent: &[Entry],
    screen: Screen,
    wake: &vpdf_pdf::jobs::Wake,
    terminal: bool,
  ) -> Self {
    let view = vpdf_ui::home::Home::new(
      crate::panel::recent_rows(recent),
      &crate::recent::entries(recent),
      screen,
      if terminal {
        vpdf_ui::home::Picker::Terminal { fzf: cmdline::on_path("fzf") }
      } else {
        vpdf_ui::home::Picker::Native
      },
    );
    Self {
      view,
      loader: Loader::new("", wake),
      graphics: Graphics::live(),
      theme: vpdf_model::theme::Theme::Default,
      settings: crate::config::Settings::default(),
    }
  }

  pub(crate) fn configure(&mut self, settings: &crate::config::Settings) {
    self.settings = settings.clone();
    self.theme = settings.theme();
    self.view.set_bindings(settings.bindings().clone());
  }

  pub(crate) fn updates(&mut self) -> bool {
    self.view.shelf_mut().is_some_and(|cards| self.loader.updates(cards))
  }

  pub(crate) fn scene<'a, S: vpdf_model::scene::Surface<'a>>(
    &mut self,
    scene: &mut S,
  ) -> std::result::Result<(), S::Error> {
    if self.theme != vpdf_model::theme::Theme::Default {
      vpdf_model::scene::Draw::push(
        scene,
        vpdf_model::scene::Op::Theme(self.theme),
      );
    }
    self
      .view
      .scene(scene, crate::state::now(), |cards| self.loader.request(cards))
  }

  pub fn draw(&mut self, out: &mut impl Write) -> Result<()> {
    write!(out, "{}{}", terminal::frame_start(), sgr(Role::Reset))?;
    self.updates();
    self.graphics.begin(out)?;
    crate::presentation::draw(out, Some(&mut self.graphics), |scene| {
      if self.theme != vpdf_model::theme::Theme::Default {
        vpdf_model::scene::Draw::push(
          scene,
          vpdf_model::scene::Op::Theme(self.theme),
        );
      }
      self
        .view
        .scene(scene, crate::state::now(), |cards| self.loader.request(cards))
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

  pub(crate) fn native_event(
    &mut self,
    event: Event,
    history: &mut History,
  ) -> Result<Outcome> {
    self.handle(event, history, false)
  }

  fn handle(
    &mut self,
    event: Event,
    history: &mut History,
    terminal: bool,
  ) -> Result<Outcome> {
    Ok(match self.view.event(event, history)? {
      Effect::Stay => Outcome::Stay,
      Effect::Open(path) => Outcome::Open(path),
      Effect::Forget(paths) => Outcome::Forget(paths),
      Effect::Quit => Outcome::Quit,
      Effect::Command(line) => return self.command(&line, terminal),
      Effect::Paste(text) => {
        crate::dnd::pdf(&text).map_or(Outcome::Stay, Outcome::Open)
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
          cmdline::complete(&before, View::Home, terminal, &names);
        self.view.complete_prompt(start, items);
        if cmdline::help_query(&before).is_some() {
          self.view.describe_prompt(|name| cmdline::describe(name, &topics));
        }
        Outcome::Stay
      }
    })
  }

  fn command(&mut self, line: &str, terminal: bool) -> Result<Outcome> {
    let resolved = commands::resolve(
      line,
      View::Home,
      |program| cmdline::available(program, terminal),
      |_| false,
    )?;
    let commands::Resolution::Command { spec, argument } = resolved else {
      return Ok(Outcome::Stay);
    };
    match spec.id {
      Id::Edit | Id::Open | Id::TabEdit | Id::TabNew
        if !argument.is_empty() =>
      {
        Ok(Outcome::Open(cmdline::expand(argument)))
      }
      Id::Edit | Id::Open | Id::TabEdit => {
        Ok(Outcome::Pick { tab: spec.id != Id::Edit })
      }
      Id::Quit | Id::QuitAll | Id::TabClose => Ok(Outcome::Quit),
      Id::Recent | Id::TabNew => Ok(Outcome::Stay),
      Id::Fzf => Ok(Outcome::Pick { tab: false }),
      Id::Version => {
        self.view.show_info(
          vpdf_ui::keys::PanelKind::Version,
          &crate::version::panel_text(terminal),
        );
        Ok(Outcome::Stay)
      }
      Id::Help | Id::HelpBeside => Ok(Outcome::Help(argument.into())),
      Id::Config => {
        crate::config::reload_command(argument)?;
        Ok(Outcome::ConfigReload)
      }
      Id::Theme => commands::theme(argument).map(Outcome::Theme),
      Id::Perf => crate::perf::switch(argument).map(Outcome::Perf),
      _ => anyhow::bail!("Open a document before :{}", spec.name),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_pdf::jobs::Wake;

  fn resolve(line: &str, terminal: bool) -> Result<Outcome> {
    let screen = crate::screen::cells(80, 24);
    Home::new(&[], screen, &Wake::none(), terminal).command(line, terminal)
  }

  fn command(line: &str) -> Result<Outcome> {
    resolve(line, true)
  }

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
      assert!(text.contains("A PDF reader with Vim keys."), "{text}");
      assert!(!text.contains("missing.pdf"), "{text}");
      assert!(!text.contains('│'), "{text}");
      if !terminal {
        assert!(text.contains(":open — Open a PDF"), "{text}");
        assert!(!text.contains(":fzf"), "{text}");
      }
    }
    Ok(())
  }

  #[test]
  fn theme_commands_work_at_home() -> Result<()> {
    use vpdf_model::theme::Theme;
    for terminal in [true, false] {
      for name in ["theme", "colorscheme"] {
        assert_eq!(
          resolve(&format!("{name} nord"), terminal)?,
          Outcome::Theme(Theme::Nord)
        );
        for argument in ["", "unknown", "nord extra"] {
          assert!(resolve(&format!("{name} {argument}"), terminal).is_err());
        }
      }
    }
    Ok(())
  }

  #[test]
  fn reload_command_works_at_home() -> Result<()> {
    assert_eq!(command("config reload")?, Outcome::ConfigReload);
    for argument in ["", "help", "bindings", "bindings normal", "licenses"] {
      assert!(command(&format!("config {argument}")).is_err());
    }
    assert_eq!(
      cmdline::candidates("config ", View::Home, true),
      (7, vec!["reload".to_owned()]),
    );
    assert!(command("config reload extra").is_err());
    assert_eq!(
      cmdline::candidates("config r", View::Home, true),
      (7, vec!["reload".to_owned()]),
    );
    Ok(())
  }

  #[test]
  fn reference_on_start_screen() -> Result<()> {
    use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};
    let screen = crate::screen::cells(160, 40);
    for (command, heading, terminal) in
      [("version", "VERSION", true), ("version", "VERSION", false)]
    {
      let mut home = Home::new(&[], screen, &Wake::none(), terminal);
      home.graphics = Graphics::default();
      let mut history = History::default();
      for ch in format!(":{command}").chars().chain(['\r']) {
        let code = if ch == '\r' { KeyCode::Enter } else { KeyCode::Char(ch) };
        assert_eq!(
          home.handle(
            Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
            &mut history,
            terminal
          )?,
          Outcome::Stay
        );
      }
      let mut out = Vec::new();
      home.draw(&mut out)?;
      let text = String::from_utf8(out)?;
      assert!(text.contains(heading));
      if command == "version" {
        assert!(
          text.contains(crate::version::text(terminal).lines().last().unwrap())
        );
      }
      if command == "version" && terminal {
        assert!(text.contains(env!("CARGO_PKG_REPOSITORY")));
      }
      home.handle(
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        &mut history,
        terminal,
      )?;
      let mut out = Vec::new();
      home.draw(&mut out)?;
      assert!(String::from_utf8(out)?.contains("A PDF reader with Vim keys."));
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
    while !home.updates() {
      anyhow::ensure!(Instant::now() < deadline, "Home card never loaded");
      std::thread::sleep(Duration::from_millis(5));
    }
    output.clear();
    home.draw(&mut output)?;
    let text = String::from_utf8_lossy(&output);
    for expected in ["Pages", "Metadata", "a=t,"] {
      assert!(text.contains(expected), "{expected}");
    }
    output.clear();
    home.draw(&mut output)?;
    assert!(!output.windows(4).any(|bytes| bytes == b"a=t,"));
    Ok(())
  }

  #[test]
  fn opening_commands_route() -> Result<()> {
    for terminal in [true, false] {
      for (name, tab) in [
        ("edit", false),
        ("e", false),
        ("ed", false),
        ("open", true),
        ("op", true),
        ("tabedit", true),
        ("tabe", true),
      ] {
        assert_eq!(resolve(name, terminal)?, Outcome::Pick { tab });
        assert_eq!(
          resolve(&format!("{name} notes.pdf"), terminal)?,
          Outcome::Open(PathBuf::from("notes.pdf"))
        );
      }
      assert_eq!(resolve("tabnew", terminal)?, Outcome::Stay);
      assert_eq!(
        resolve("tabnew notes.pdf", terminal)?,
        Outcome::Open(PathBuf::from("notes.pdf"))
      );
    }
    Ok(())
  }

  #[test]
  fn native_commands_offer_picker() -> Result<()> {
    assert_eq!(resolve("fzf", false)?, Outcome::Pick { tab: false });
    let (_, items) = cmdline::candidates("fz", View::Home, false);
    assert!(items.iter().any(|item| item.trim() == "fzf"));
    Ok(())
  }

  #[test]
  fn help_commands_and_shortcuts() -> Result<()> {
    use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};
    for terminal in [true, false] {
      assert_eq!(
        resolve("help config", terminal)?,
        Outcome::Help("config".into())
      );
      assert_eq!(resolve("h", terminal)?, Outcome::Help(String::new()));
      for sequence in ["g?", "<F1>"] {
        let mut home = Home::new(
          &[],
          crate::screen::cells(120, 40),
          &Wake::none(),
          terminal,
        );
        let mut history = History::default();
        let mut outcome = Outcome::Stay;
        for &key in vpdf_ui::bindings::Sequence::parse(sequence)?.keys() {
          outcome = home.handle(Event::Key(key), &mut history, terminal)?;
        }
        assert_eq!(outcome, Outcome::Help(String::new()));
        home.handle(
          Event::Key(KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE)),
          &mut history,
          terminal,
        )?;
        home.handle(
          Event::Paste("help conf".into()),
          &mut history,
          terminal,
        )?;
        home.handle(
          Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
          &mut history,
          terminal,
        )?;
        let outcome = home.handle(
          Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
          &mut history,
          terminal,
        )?;
        assert_eq!(outcome, Outcome::Help("config".into()));
      }
    }
    Ok(())
  }

  #[test]
  fn start_commands_open_or_quit() -> Result<()> {
    assert_eq!(command("e a.pdf")?, Outcome::Open("a.pdf".into()));
    assert_eq!(command("tabnew  b.pdf ")?, Outcome::Open("b.pdf".into()));
    assert_eq!(command("q")?, Outcome::Quit);
    assert_eq!(command("")?, Outcome::Stay);
    assert_eq!(command("e")?, Outcome::Pick { tab: false });
    assert!(command("toc").is_err());
    assert!(
      cmdline::candidates("hel", View::Home, true)
        .1
        .iter()
        .any(|item| item.trim() == "help")
    );
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
    assert_eq!(home.event(key(KeyCode::Enter), &mut history)?, Outcome::Stay);
    assert_eq!(history.commands().last().map(String::as_str), Some("recent"));
    Ok(())
  }
}
