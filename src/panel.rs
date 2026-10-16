use crate::Result;
use crate::app::App;
use crate::document::Target;
use crate::keys::{Action, Input, PanelKind};
use crate::motion::Motion;
use crate::terminal;
use crate::text::Position;
use std::io::Write;

#[derive(Clone)]
pub enum Destination {
  Position(Position),
  Link(Target),
  None,
}

pub struct Row {
  pub label: String,
  pub destination: Destination,
}

pub struct Panel {
  pub kind: PanelKind,
  pub rows: Vec<Row>,
  pub selected: usize,
  pub focused: bool,
}

impl Panel {
  pub fn draw(
    &self,
    out: &mut impl Write,
    width: u16,
    height: u16,
  ) -> Result<()> {
    let title = format!("{:?}  gq close", self.kind);
    write!(
      out,
      "\x1b[1;1H\x1b[1m{}\x1b[0m",
      terminal::clean(&title, usize::from(width))
    )?;
    let count = height.saturating_sub(2) as usize;
    let start = self.selected.saturating_sub(count / 2);
    for (row, item) in self.rows.iter().skip(start).take(count).enumerate() {
      let style = if start + row == self.selected { "7" } else { "0" };
      write!(
        out,
        "\x1b[{};1H\x1b[{style}m{}\x1b[0m",
        row + 2,
        terminal::clean(&item.label, width.saturating_sub(1) as usize)
      )?;
    }
    Ok(())
  }
}

impl App {
  pub fn track_outline(&mut self) {
    let Some(panel) = &mut self.panel else { return };
    if panel.kind != PanelKind::Outline || panel.focused {
      return;
    }
    let current = self.reader.cursor.page;
    if let Some(index) = panel.rows.iter().rposition(|row| {
      matches!(row.destination,
        Destination::Link(Target::Page(page) | Target::Point { page, .. })
        if page <= current)
    }) {
      panel.selected = index;
    }
  }

  pub fn open_panel(&mut self, kind: PanelKind) -> Result<()> {
    if kind == PanelKind::Text {
      self.preview = Some(self.text_mode);
      self.text_mode = true;
      return Ok(());
    }
    if let Some(panel) = &mut self.panel
      && panel.kind == kind
    {
      if kind == PanelKind::Outline {
        panel.focused = true;
      } else {
        self.close_panel()?;
      }
      return Ok(());
    }
    let rows = self.panel_rows(kind)?;
    self.panel = Some(Panel { kind, rows, selected: 0, focused: true });
    self.left = (self.screen.columns / 3)
      .clamp(1, 50)
      .min(self.screen.columns.saturating_sub(12));
    self.resize()
  }

  pub fn close_panel(&mut self) -> Result<()> {
    self.panel = None;
    self.left = 0;
    if let Some(previous) = self.preview.take() {
      self.text_mode = previous;
    }
    self.resize()
  }

  pub fn panel_rows(&mut self, kind: PanelKind) -> Result<Vec<Row>> {
    Ok(match kind {
      PanelKind::Outline => self
        .doc
        .outline
        .iter()
        .map(|item| Row {
          label: format!("{}{}", "  ".repeat(item.depth.min(12)), item.title),
          destination: Destination::Link(item.target.clone()),
        })
        .collect(),
      PanelKind::Results => self
        .search
        .hits
        .iter()
        .map(|hit| Row {
          label: format!("{}: {}", hit.first.page.0 + 1, hit.excerpt),
          destination: Destination::Position(hit.first),
        })
        .collect(),
      PanelKind::Links => self
        .doc
        .links(self.reader.cursor.page)?
        .iter()
        .map(|link| Row {
          label: match &link.target {
            Target::Page(page) | Target::Point { page, .. } => {
              format!("Page {}", page.0 + 1)
            }
            Target::Uri(uri) => uri.clone(),
          },
          destination: Destination::Link(link.target.clone()),
        })
        .collect(),
      PanelKind::Help => help()
        .into_iter()
        .map(|label| Row { label, destination: Destination::None })
        .collect(),
      PanelKind::Info => self
        .info()
        .lines()
        .map(|line| Row {
          label: line.to_owned(),
          destination: Destination::None,
        })
        .collect(),
      PanelKind::Text => Vec::new(),
    })
  }

  pub fn panel_input(&mut self, input: Input) -> Result<bool> {
    let Some(panel) = &mut self.panel else { return Ok(false) };
    if !panel.focused {
      return Ok(false);
    }
    let last = panel.rows.len().saturating_sub(1);
    match input.action {
      Action::Motion(Motion::Down) => {
        panel.selected = panel.selected.saturating_add(input.count).min(last)
      }
      Action::Motion(Motion::Up) => {
        panel.selected = panel.selected.saturating_sub(input.count)
      }
      Action::Motion(Motion::First) => panel.selected = 0,
      Action::Motion(Motion::Last) => panel.selected = last,
      Action::Scroll(amount) => {
        panel.selected = (panel.selected as i64
          + (amount * f32::from(self.screen.rows) * input.count as f32) as i64)
          .clamp(0, last as i64) as usize
      }
      Action::Quit | Action::ClosePanel => {
        self.close_panel()?;
      }
      Action::Cancel => {
        panel.focused = false;
      }
      Action::Follow | Action::Swap => {
        self.panel_follow(input.action)?;
      }
      _ => return Ok(false),
    }
    Ok(true)
  }

  fn panel_follow(&mut self, action: Action) -> Result<()> {
    let Some(panel) = &self.panel else { return Ok(()) };
    let Some(row) = panel.rows.get(panel.selected) else { return Ok(()) };
    let destination = row.destination.clone();
    let outline = panel.kind == PanelKind::Outline;
    match destination {
      Destination::Position(position) => {
        self.reader.jump(position, &mut self.doc)?
      }
      Destination::Link(target) => self.follow(&target)?,
      Destination::None => return Ok(()),
    }
    if action == Action::Follow {
      if outline {
        if let Some(panel) = &mut self.panel {
          panel.focused = false;
        }
      } else {
        self.close_panel()?;
      }
    }
    self.reader.scroll = self.layout.pages[self.reader.cursor.page.0].top;
    self.ensure_cursor()
  }

  pub fn info(&self) -> String {
    format!(
      "{}\nPages: {}\nText cache: {} KiB\nImages: {} KiB\n\
      Transferred: {} KiB\nLast tile: {:.3} ms\nSearch hits: {}",
      self.path.display(),
      self.doc.pages.len(),
      self.doc.cache_bytes() / 1024,
      self.graphics.bytes() / 1024,
      self.graphics.uploaded / 1024,
      self.render_ms,
      self.search.hits.len()
    )
  }
}

fn help() -> Vec<String> {
  let keys = include_str!("help.txt").lines().map(str::to_owned);
  let commands = crate::cmdline::COMMANDS.iter().map(|spec| {
    let usage = if spec.usage.is_empty() { "" } else { " " };
    format!(":{}{usage}{}: {}", spec.name, spec.usage, spec.help)
  });
  keys.chain(commands).collect()
}
