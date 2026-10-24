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

pub(crate) fn scene<'a>(
  scene: &mut impl Draw<'a>,
  screen: Screen,
  at: At,
  picker: Picker,
) {
  let (action, label) = match picker {
    Picker::Terminal { fzf: true } => (":fzf", "Find a PDF"),
    Picker::Terminal { fzf: false } => {
      (":help install", "fzf unavailable; optional helpers")
    }
    Picker::Native => (":open", "Open a PDF"),
  };
  let mut commands = [
    (action, label),
    (":edit <file>", "Open by path"),
    (":help", "Learn the reader"),
    (":q", "Quit"),
  ];
  if matches!(picker, Picker::Terminal { fzf: false }) {
    commands.swap(0, 1);
  }
  let height = screen.rows.saturating_sub(1);
  let compact = height < 9 || screen.columns < 44;
  let gap = u16::from(!compact);
  let lines = 6 + gap;
  let top = height.saturating_sub(lines) / 3;
  let width = commands
    .iter()
    .map(|(command, label)| command.width() + label.width() + 3)
    .chain(["A PDF reader with Vim keys.".len()])
    .max()
    .unwrap_or(0)
    .min(usize::from(screen.columns));
  let column = (usize::from(screen.columns) - width) / 2;
  let mut draw = |row: u16, role, text: &str| {
    if row >= height || screen.columns == 0 {
      return;
    }
    scene.extend([
      Op::Move {
        row: usize::from(at.row) + usize::from(row),
        column: usize::from(at.column) + column,
      },
      Op::Style(role),
      Op::Text(Text::new(vpdf_model::text::clean(text, width))),
      Op::Style(Role::Reset),
    ]);
  };
  draw(top, Role::Heading, "Vpdf");
  // Short windows prioritize opening commands.
  let intro = u16::from(height >= lines);
  if intro != 0 {
    draw(top + 1, Role::Muted, "A PDF reader with Vim keys.");
  }
  for (index, (command, label)) in commands.iter().enumerate() {
    let text = if compact {
      format!("{command}  {label}")
    } else {
      format!("{command} — {label}")
    };
    draw(top + 1 + intro + gap + index as u16, Role::Bar, &text);
  }
}
