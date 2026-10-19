use crate::Result;
use crate::cmdline;
use crate::recent::Loader;
use crate::state::Entry;
use std::io::Write;
use std::path::PathBuf;
use vpdf_terminal::decode::Event;
use vpdf_terminal::graphics::Graphics;
use vpdf_terminal::terminal;
use vpdf_ui::commands::{self, View};
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
    let view = vpdf_ui::home::Home::new(
      crate::panel::recent_rows(recent),
      &crate::recent::entries(recent),
      screen,
    );
    let wake = vpdf_pdf::jobs::Wake::new(vpdf_terminal::input::wake);
    Self { view, loader: Loader::new("", &wake), graphics: Graphics::live() }
  }

  pub(crate) fn updates(&mut self) -> bool {
    self.view.shelf_mut().is_some_and(|cards| self.loader.updates(cards))
  }

  pub fn draw(&mut self, out: &mut impl Write) -> Result<()> {
    write!(out, "{}{}", terminal::frame_start(), vpdf_ui::theme::RESET)?;
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
    event: Event,
    history: &mut History,
  ) -> Result<Outcome> {
    Ok(match self.view.event(event, history)? {
      Effect::Stay => Outcome::Stay,
      Effect::Open(path) => Outcome::Open(path),
      Effect::Quit => Outcome::Quit,
      Effect::Command(line) => return command(&line),
      Effect::Paste(text) => {
        crate::dnd::pdf(&text).map_or(Outcome::Stay, Outcome::Open)
      }
      Effect::Editor(text) => {
        let edited = crate::editor::edit(&text)?;
        self.view.replace_prompt(&edited);
        Outcome::Stay
      }
      Effect::Complete(before) => {
        let (start, items) = cmdline::candidates(&before);
        self.view.complete_prompt(start, items);
        Outcome::Stay
      }
    })
  }
}

/// Commands that make sense before any document is open.
fn command(line: &str) -> Result<Outcome> {
  let (name, argument) = commands::parse(line);
  if name.is_empty() {
    return Ok(Outcome::Stay);
  }
  let spec = commands::lookup(name, cmdline::on_path)
    .ok_or_else(|| anyhow::anyhow!("Unknown command: {name}"))?;
  let name = spec.name;
  anyhow::ensure!(spec.available(View::Home), "Open a document before :{name}");
  match name {
    "edit" | "tabedit" | "tabnew" if !argument.is_empty() => {
      Ok(Outcome::Open(cmdline::expand(argument)))
    }
    "edit" | "tabedit" => anyhow::bail!("File required"),
    "quit" | "qall" | "tabclose" => Ok(Outcome::Quit),
    "recent" | "tabnew" => Ok(Outcome::Stay),
    "fzf" => Ok(Outcome::Pick),
    "perf" => crate::perf::switch(argument).map(Outcome::Perf),
    name => anyhow::bail!("Open a document before :{name}"),
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
  fn start_commands_open_or_quit() -> Result<()> {
    assert_eq!(command("e a.pdf")?, Outcome::Open("a.pdf".into()));
    assert_eq!(command("tabnew  b.pdf ")?, Outcome::Open("b.pdf".into()));
    assert_eq!(command("q")?, Outcome::Quit);
    assert_eq!(command("")?, Outcome::Stay);
    assert!(command("e").is_err());
    assert!(command("toc").is_err());
    Ok(())
  }
}
