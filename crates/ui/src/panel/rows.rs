use super::{
  Destination, Row, RowKind, Style, UnicodeWidthChar, UnicodeWidthStr,
};
use vpdf_model::scene::{Draw, Op};
use vpdf_model::style::Role;

impl Row {
  pub fn follows(&self) -> bool {
    !matches!(self.destination, Destination::None)
  }

  pub fn new(label: String, detail: String, destination: Destination) -> Self {
    Self {
      label,
      detail,
      key: String::new(),
      kind: RowKind::Plain,
      depth: 0,
      style: Style::Plain,
      destination,
    }
  }

  /// Reads `# heading` and `keys: description` lines.
  pub fn parse(line: &str) -> Self {
    let style = if line.starts_with("# ") {
      Style::Heading
    } else if line.contains(": ") {
      Style::Key
    } else {
      Style::Plain
    };
    let label = line.strip_prefix("# ").unwrap_or(line).to_owned();
    Self { style, ..Self::new(label, String::new(), Destination::None) }
  }
}

/// Widest `keys:` column of `rows`.
pub fn key_width(rows: &[Row], inner: usize) -> usize {
  rows
    .iter()
    .filter(|row| {
      matches!(row.style, Style::Key | Style::ModeKey | Style::CommandKey)
    })
    .filter_map(|row| row.label.split_once(": "))
    .map(|(key, _)| key.width())
    .max()
    .unwrap_or(0)
    .min(inner / 2)
}

impl Row {
  pub fn scene<'a>(
    &self,
    scene: &mut impl Draw<'a>,
    base: Option<Role>,
    room: usize,
    keys: usize,
    query: &str,
  ) -> usize {
    let plain = accented(base);
    match (self.style, self.label.split_once(": ")) {
      (Style::Key | Style::ModeKey | Style::CommandKey, Some((key, text))) => {
        let key = vpdf_model::text::clean(key, room);
        let pad = (keys.max(key.width()) + 2 - key.width())
          .min(room.saturating_sub(key.width()));
        let text =
          vpdf_model::text::clean(text, room.saturating_sub(key.width() + pad));
        let style = plain.then_some(Role::Key);
        let mut written = 0;
        for (start, end, token) in crate::help::key_styles(&key, self.style) {
          marked(scene, &key[written..start], query, base, style);
          marked(scene, &key[start..end], query, base, Some(token));
          written = end;
        }
        marked(scene, &key[written..], query, base, style);
        styles(scene, base, None);
        scene.push(Op::Spaces(pad));
        marked(scene, &text, query, base, None);
        key.width() + pad + text.width()
      }
      (Style::Path, _) => {
        let split = self.label.rfind('/').map_or(0, |slash| slash + 1);
        let (parent, name) = self.label.split_at(split);
        let name = vpdf_model::text::clean(name, room);
        let parent = tail(parent, room.saturating_sub(name.width()));
        let muted = plain.then_some(Role::Muted);
        marked(scene, &parent, query, base, muted);
        marked(scene, &name, query, base, Some(Role::File));
        styles(scene, base, None);
        parent.width() + name.width()
      }
      (Style::Muted, _) => {
        let text = vpdf_model::text::clean(&self.label, room);
        marked(scene, &text, query, base, plain.then_some(Role::Muted));
        text.width()
      }
      (Style::Heading, _) => {
        let text = vpdf_model::text::clean(&self.label, room);
        marked(scene, &text, query, base, Some(Role::Heading));
        styles(scene, base, None);
        text.width()
      }
      _ => {
        let text = vpdf_model::text::clean(&self.label, room);
        marked(scene, &text, query, base, None);
        text.width()
      }
    }
  }
}

/// Unselected rows show accent colors.
pub(super) fn accented(base: Option<Role>) -> bool {
  matches!(base, Some(Role::Reset | Role::Dim | Role::Hover | Role::HoverDim))
}

impl RowKind {
  pub fn glyph(self) -> Option<(char, Role)> {
    Some(match self {
      Self::Plain => return None,
      Self::Figure => ('▣', Role::Heading),
      Self::Section => ('§', Role::Heading),
      Self::Table => ('▤', Role::Mode),
      Self::Mention => ('¶', Role::Muted),
      Self::Search => ('⌕', Role::Key),
      Self::Note => ('✎', Role::Key),
      Self::Underline => ('‗', Role::Argument),
      Self::Highlight => ('▇', Role::Key),
    })
  }
}

/// `text` within `columns`, cut from the left behind an ellipsis.
fn tail(text: &str, columns: usize) -> String {
  let text: String = crate::text::printable(text).collect();
  if text.width() <= columns {
    return text;
  }
  let Some(room) = columns.checked_sub(1) else { return String::new() };
  let mut width = 0;
  let kept: Vec<char> = text
    .chars()
    .rev()
    .take_while(|ch| {
      width += ch.width().unwrap_or(0);
      width <= room
    })
    .collect();
  std::iter::once('…').chain(kept.into_iter().rev()).collect()
}

/// The gutter cell: `▌` on the current row.
pub(super) fn gutter<'a>(scene: &mut impl Draw<'a>, current: bool, base: Role) {
  if current {
    scene.push(Op::Style(Role::Heading));
    scene.text("▌");
    scene.push(Op::Style(base));
  } else {
    scene.text(" ");
  }
}

pub(super) fn styles<'a>(
  scene: &mut impl Draw<'a>,
  base: Option<Role>,
  accent: Option<Role>,
) {
  for role in base.into_iter().chain(accent) {
    scene.push(Op::Style(role));
  }
}

pub(super) fn marked<'a>(
  scene: &mut impl Draw<'a>,
  text: &str,
  query: &str,
  base: Option<Role>,
  accent: Option<Role>,
) {
  styles(scene, base, accent);
  let text = if matches!(base, Some(Role::Hover | Role::HoverDim)) {
    let title = text.trim_start_matches(' ');
    let indent = &text[..text.len() - title.len()];
    scene.push(Op::UnderlineOff);
    scene.text(indent);
    styles(scene, base, accent);
    title
  } else {
    text
  };
  let mut written = 0;
  for (start, end) in crate::panel::search::ranges(text, query) {
    scene.text(&text[written..start]);
    scene.push(Op::Style(Role::TextMatch));
    scene.text(&text[start..end]);
    styles(scene, base, accent);
    written = end;
  }
  scene.text(&text[written..]);
}
