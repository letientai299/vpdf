use crate::layout::Screen;
use unicode_width::UnicodeWidthStr;
use vpdf_model::geometry::At;
use vpdf_model::scene::{Draw, Op, Text};
use vpdf_model::style::Role;

#[derive(Clone, Copy)]
pub enum Picker {
  Terminal { fzf: bool },
  Native,
}

impl Picker {
  pub fn command(self) -> &'static str {
    match self {
      Self::Terminal { fzf: true } => "fzf",
      Self::Terminal { fzf: false } | Self::Native => "open",
    }
  }
}

pub(crate) fn scene<'a>(
  scene: &mut impl Draw<'a>,
  screen: Screen,
  at: At,
  picker: Picker,
) {
  commands(scene, screen, at, picker, 0);
}

pub(crate) fn commands<'a>(
  scene: &mut impl Draw<'a>,
  screen: Screen,
  at: At,
  picker: Picker,
  image_rows: u16,
) {
  let opening = format!(":{}", picker.command());
  let commands = [
    (opening.as_str(), "Open a PDF"),
    (":edit <file>", "Open by path"),
    (":help", "Learn the reader"),
    (":q", "Quit"),
  ];
  let height = screen.rows.saturating_sub(1);
  let decoration = if height >= 7 { image_rows.max(1) } else { 0 };
  let lines = decoration + 4;
  let top = height.saturating_sub(lines) / 2;
  let keys =
    commands.iter().map(|(command, _)| command.width()).max().unwrap_or(0);
  let width =
    (keys + 2 + "Learn the reader".width()).min(usize::from(screen.columns));
  let column = (usize::from(screen.columns) - width) / 2;
  if decoration > 0 && image_rows == 0 {
    let title_column = screen.columns.saturating_sub(4) / 2;
    crate::paint::move_to(scene, at, top, title_column);
    scene.push(Op::Style(Role::Heading));
    scene.push(Op::Text(Text::fit("Vpdf", usize::from(screen.columns))));
  }
  for (index, (command, label)) in commands.iter().enumerate() {
    let row = top + decoration + index as u16;
    if row >= height {
      break;
    }
    crate::paint::move_to(scene, at, row, column);
    scene.push(Op::Style(Role::Key));
    let command = vpdf_model::text::clean(command, width);
    scene.text(&command);
    let used = command.width();
    let padding =
      (keys + 2).saturating_sub(used).min(width.saturating_sub(used));
    scene.push(Op::Spaces(padding));
    scene.push(Op::Style(Role::Muted));
    scene
      .push(Op::Text(Text::fit(label, width.saturating_sub(used + padding))));
    scene.push(Op::Style(Role::Reset));
  }
}
