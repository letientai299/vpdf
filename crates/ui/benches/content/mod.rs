use anyhow::Result;
use std::hint::black_box;
use vpdf_model::document::{Heading, Target};
use vpdf_model::text::Position;
use vpdf_ui::panel::{Row, links};

#[derive(serde::Deserialize)]
struct Prepared {
  headings: Vec<(String, usize, Target)>,
  hits: Vec<(Position, Position, String)>,
}

pub fn rows() -> Result<()> {
  let prepared: Prepared = super::fixture("scrollbar")?;
  let pages = super::fixtures::text()?;
  let headings = headings(prepared.headings);
  anyhow::ensure!(
    !headings.is_empty() && !prepared.hits.is_empty(),
    "Missing panel content"
  );
  let name = |page: vpdf_model::text::PageId| pages.metadata[page.0].0.as_str();
  let outlines = || {
    headings
      .iter()
      .map(|heading| {
        let label = match heading.target {
          Target::Page(page) | Target::Point { page, .. } => {
            name(page).to_owned()
          }
          Target::Uri(_) => String::new(),
        };
        Row::outline(heading, label)
      })
      .collect::<Vec<_>>()
  };
  let jumps = || {
    prepared
      .hits
      .iter()
      .map(|(position, _, _)| {
        let line = pages.text[position.page.0].lines.get(position.line);
        Row::jump(*position, None, line, name(position.page))
      })
      .collect::<Vec<_>>()
  };
  let bytes: Vec<_> = outlines()
    .into_iter()
    .chain(jumps())
    .flat_map(|r| format!("{}\n{}\n", r.label, r.detail).into_bytes())
    .collect();
  println!("panel content output {}", super::hash::digest(&bytes));
  super::measure("Row::outline prepared", || {
    black_box(outlines());
  })?;
  super::measure("Row::jump prepared", || {
    black_box(jumps());
  })?;
  super::measure("links::destination prepared", || {
    for heading in &headings {
      black_box(links::destination(
        &heading.target,
        &headings,
        pages.metadata.len(),
        name,
        "",
      ));
    }
  })
}

fn headings(prepared: Vec<(String, usize, Target)>) -> Vec<Heading> {
  prepared
    .into_iter()
    .map(|(title, depth, target)| Heading { title, depth, target })
    .collect()
}

struct Outlined<'a> {
  pages: &'a super::fixtures::TextPages,
  headings: &'a [Heading],
}

impl vpdf_reader::reader::TextSource for Outlined<'_> {
  fn page_count(&self) -> usize {
    self.pages.text.len()
  }
  fn page_text(
    &mut self,
    page: vpdf_model::text::PageId,
  ) -> Result<std::sync::Arc<vpdf_model::text::Text>> {
    Ok(std::sync::Arc::clone(&self.pages.text[page.0]))
  }
}

impl vpdf_ui::source::Pages for Outlined<'_> {
  fn count(&self) -> usize {
    self.pages.metadata.len()
  }
  fn bounds(&self, page: vpdf_model::text::PageId) -> vpdf_model::text::Bounds {
    self.pages.metadata[page.0].1
  }
  fn label(&self, page: vpdf_model::text::PageId) -> &str {
    &self.pages.metadata[page.0].0
  }
}

impl vpdf_ui::source::Outline for Outlined<'_> {
  fn outline(&self) -> &[Heading] {
    self.headings
  }
}

pub fn tracking() -> Result<()> {
  use vpdf_model::geometry::Screen;
  use vpdf_ui::keys::PanelKind;
  use vpdf_ui::layout::Layout;
  use vpdf_ui::panel::Panel;
  use vpdf_ui::reading::{Frame, Outline, Pane};
  use vpdf_ui::text_view::TextView;

  let prepared: Prepared = super::fixture("scrollbar")?;
  let pages = super::fixtures::text()?;
  let headings = headings(prepared.headings);
  let target = headings
    .iter()
    .find_map(|heading| match heading.target {
      Target::Point { page, x, y } => Some((page, x, y)),
      _ => None,
    })
    .ok_or_else(|| anyhow::anyhow!("Missing outline point"))?;
  let screen = Screen {
    columns: 120,
    rows: 24,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  for text in [false, true] {
    for cursor in [false, true] {
      for landed in [false, true] {
        let mut reading = Pane::default();
        if cursor {
          reading.enter_cursor();
        }
        let mut layout =
          Layout::new(pages.metadata.iter().map(|(_, bounds)| *bounds), screen);
        let mut text_view = TextView::default();
        let frame = Frame {
          pane: vpdf_ui::paint::Frame {
            screen,
            at: vpdf_model::geometry::At::default(),
            left: 0,
            text,
          },
          visible: screen,
        };
        let source = || Outlined { pages: &pages, headings: &headings };
        if landed {
          let (page, x, y) = target;
          let mut view =
            reading.bind(&mut layout, &mut text_view, frame, source());
          let position = view.destination(page, Some((x, y)))?;
          view.land_at(position)?;
          reading.aim(Target::Point { page, x, y });
        }
        let rows = headings
          .iter()
          .map(|heading| Row::outline(heading, String::new()))
          .collect();
        let mut panel = Panel::new(PanelKind::Outline, rows);
        panel.resize(screen.rows);
        panel.focus(false);
        let mut outline = Outline::default();
        super::measure(
          &format!(
            "Outline::track text={text} cursor={cursor} landed={landed}"
          ),
          || {
            let mut view =
              reading.bind(&mut layout, &mut text_view, frame, source());
            outline.track(&mut view, Some(&mut panel));
            black_box((outline.current(), panel.selected()));
          },
        )?;
      }
    }
  }
  Ok(())
}

pub fn input() -> Result<()> {
  use vpdf_reader::motion::Motion;
  use vpdf_ui::keys::{Action, Input, PanelKind};
  use vpdf_ui::panel::Panel;
  let prepared: Prepared = super::fixture("scrollbar")?;
  let headings = headings(prepared.headings);
  anyhow::ensure!(headings.len() > 2, "Missing panel navigation rows");
  let rows = headings
    .iter()
    .map(|heading| Row::outline(heading, String::new()))
    .collect();
  let mut panel = Panel::new(PanelKind::Outline, rows);
  let input =
    |action| Input { action, count: 2, explicit: true, register: 'a' };
  super::measure("Panel::input navigation", || {
    for motion in [Motion::Down, Motion::Up] {
      black_box(panel.input(black_box(input(Action::Motion(motion))), 24));
    }
  })?;
  super::measure("Panel::input copy", || {
    black_box(panel.input(black_box(input(Action::YankLine)), 24));
  })?;
  let search = vpdf_ui::find::Search::default();
  super::measure("Panel::follow outline", || {
    black_box(panel.follow(black_box(&search)));
  })?;
  super::measure("Panel::refocus outline", || {
    black_box(panel.refocus());
  })?;
  let mouse = vpdf_model::input::MouseEvent {
    kind: vpdf_model::input::MouseEventKind::Down(
      vpdf_model::input::MouseButton::Left,
    ),
    column: 2,
    row: 1,
    modifiers: vpdf_model::input::KeyModifiers::NONE,
  };
  super::measure("Press::activate repeat", || {
    let press = panel.press(black_box(mouse));
    black_box(press.activate(&mut panel, 2));
  })
}

pub fn help() -> Result<()> {
  use vpdf_model::geometry::At;
  use vpdf_ui::keys::PanelKind;
  use vpdf_ui::panel::{Panel, help};
  super::measure("panel::help reference", || {
    black_box(help(|_| true));
  })?;
  for width in [30, 64] {
    let rows = help(|_| true).iter().map(|line| Row::parse(line)).collect();
    let mut panel = Panel::new(PanelKind::Help, rows);
    let mut out = Vec::new();
    panel.scene_list(&mut out, At::default(), width, 24);
    super::measure(&format!("Panel::help scene {width}x24"), || {
      out.clear();
      panel.scene_list(&mut out, At::default(), width, 24);
      black_box(&out);
    })?;
  }
  Ok(())
}

pub fn definition() -> Result<()> {
  use vpdf_model::text::PageId;
  use vpdf_search::reference::Reference;
  use vpdf_ui::find::Definition;
  let prepared: Prepared = super::fixture("scrollbar")?;
  let pages = super::fixtures::text()?;
  let headings = headings(prepared.headings);
  let mut positions = Vec::new();
  'pages: for (page, text) in pages.text.iter().enumerate() {
    for (line_index, line) in text.lines.iter().enumerate() {
      if let Some(glyph) = (0..line.glyphs.len())
        .find(|&glyph| Reference::at_glyph(line, glyph).is_some())
      {
        positions.push(Position {
          page: PageId(page),
          line: line_index,
          glyph,
        });
        if positions.len() == 16 {
          break 'pages;
        }
      }
    }
  }
  anyhow::ensure!(!positions.is_empty(), "Missing document references");
  let mut source = Outlined { pages: &pages, headings: &headings };
  let resolved: Vec<_> = positions
    .iter()
    .map(|position| Definition::resolve(*position, None, &mut source))
    .collect::<Result<_>>()?;
  println!(
    "definition targets {} output {}",
    positions.len(),
    super::hash::digest(format!("{resolved:?}").as_bytes())
  );
  super::measure("Definition::resolve batch", || {
    for position in &positions {
      black_box(
        Definition::resolve(black_box(*position), None, &mut source)
          .expect("prepared reference"),
      );
    }
  })
}
