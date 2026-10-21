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
  Perf(Option<bool>),
  Open(PathBuf),
  Pick,
  Quit,
}

pub struct Home {
  pub view: vpdf_ui::home::Home,
  loader: Loader,
  graphics: Graphics,
}

impl Home {
  pub fn new(recent: &[Entry], screen: Screen) -> Self {
    Self::with_wake(
      recent,
      screen,
      &vpdf_pdf::jobs::Wake::new(vpdf_terminal::input::wake),
    )
  }

  pub(crate) fn with_wake(
    recent: &[Entry],
    screen: Screen,
    wake: &vpdf_pdf::jobs::Wake,
  ) -> Self {
    let view = vpdf_ui::home::Home::new(
      crate::panel::recent_rows(recent),
      &crate::recent::entries(recent),
      screen,
    );
    Self { view, loader: Loader::new("", wake), graphics: Graphics::live() }
  }

  pub(crate) fn updates(&mut self) -> bool {
    self.view.shelf_mut().is_some_and(|cards| self.loader.updates(cards))
  }

  pub(crate) fn scene<'a, S: vpdf_model::scene::Surface<'a>>(
    &mut self,
    scene: &mut S,
  ) -> std::result::Result<(), S::Error> {
    self
      .view
      .scene(scene, crate::state::now(), |cards| self.loader.request(cards))
  }

  pub fn draw(&mut self, out: &mut impl Write) -> Result<()> {
    write!(out, "{}{}", terminal::frame_start(), sgr(Role::Reset))?;
    self.updates();
    self.graphics.begin(out)?;
    self.view.draw(out, &mut self.graphics, crate::state::now(), |cards| {
      self.loader.request(cards);
    })?;
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
      Effect::Quit => Outcome::Quit,
      Effect::Command(line) => return resolve(&line, terminal),
      Effect::Paste(text) => {
        crate::dnd::pdf(&text).map_or(Outcome::Stay, Outcome::Open)
      }
      Effect::Editor(text) => {
        let edited = crate::programs::edit(&text, terminal)?;
        self.view.replace_prompt(&edited);
        Outcome::Stay
      }
      Effect::Complete(before) => {
        let (start, items) = cmdline::candidates(&before, View::Home, terminal);
        self.view.complete_prompt(start, items);
        Outcome::Stay
      }
    })
  }
}

/// Commands that make sense before any document is open.
#[cfg(test)]
fn command(line: &str) -> Result<Outcome> {
  resolve(line, true)
}

fn resolve(line: &str, terminal: bool) -> Result<Outcome> {
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
    Id::Edit | Id::TabEdit | Id::TabNew if !argument.is_empty() => {
      Ok(Outcome::Open(cmdline::expand(argument)))
    }
    Id::Edit | Id::TabEdit => anyhow::bail!("File required"),
    Id::Quit | Id::QuitAll | Id::TabClose => Ok(Outcome::Quit),
    Id::Recent | Id::TabNew => Ok(Outcome::Stay),
    Id::Fzf => Ok(Outcome::Pick),
    Id::Perf => crate::perf::switch(argument).map(Outcome::Perf),
    _ => anyhow::bail!("Open a document before :{}", spec.name),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn home_loads_recent_card() -> Result<()> {
    use std::time::{Duration, Instant};
    let path = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
    let entry = crate::state::tests::entry(&path.to_string_lossy(), 0, 0);
    let screen = Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut home = Home::new(&[entry], screen);
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
  fn native_commands_offer_picker() -> Result<()> {
    assert_eq!(resolve("fzf", false)?, Outcome::Pick);
    let (_, items) = cmdline::candidates("fz", View::Home, false);
    assert!(items.iter().any(|item| item.trim() == "fzf"));
    Ok(())
  }

  #[test]
  fn start_commands_open_or_quit() -> Result<()> {
    assert_eq!(command("e a.pdf")?, Outcome::Open("a.pdf".into()));
    assert_eq!(command("tabnew  b.pdf ")?, Outcome::Open("b.pdf".into()));
    assert_eq!(command("q")?, Outcome::Quit);
    assert_eq!(command("")?, Outcome::Stay);
    assert!(command("e").is_err());
    assert!(command("toc").is_err());
    Ok(())
  }

  #[test]
  fn start_completion_uses_home_scope() -> Result<()> {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use vpdf_terminal::decode::Event;
    let screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut home = Home::new(&[], screen);
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
