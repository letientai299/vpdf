use super::{Destination, Row, RowKind, Style, UnicodeWidthStr};
use std::ops::Range;
use vpdf_model::scene::{Draw, Op};
use vpdf_model::style::Role;
use vpdf_model::text::{clipped, tail};

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
      (Style::Excerpt { start, end }, _) => {
        let (text, matched) = excerpt(&self.label, start..end, room);
        marked(scene, &text[..matched.start], query, base, None);
        let hit = Some(Role::TextMatch);
        marked(scene, &text[matched.clone()], query, base, hit);
        marked(scene, &text[matched.end..], query, base, None);
        styles(scene, base, None);
        text.width()
      }
      (Style::Document | Style::Muted, _) => {
        let role =
          if self.style == Style::Muted { Role::Muted } else { Role::Argument };
        let text = vpdf_model::text::clean(&self.label, room);
        marked(scene, &text, query, base, plain.then_some(role));
        styles(scene, base, None);
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

impl Row {
  /// The kind's glyph, in the annotation's own color when it has one.
  pub(super) fn glyph(&self) -> Option<(char, Role)> {
    let (glyph, role) = self.kind.glyph()?;
    let color = match &self.destination {
      Destination::Annotation { annotation, .. } => annotation.color,
      _ => None,
    };
    Some((glyph, color.map_or(role, Role::Swatch)))
  }

  /// An annotation's note when the label quotes other text.
  pub(super) fn note(&self) -> Option<&str> {
    let Destination::Annotation { annotation, .. } = &self.destination else {
      return None;
    };
    let note = annotation.contents.as_str();
    (!note.is_empty() && note != self.label).then_some(note)
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
      Self::Link => ('↗', Role::Mode),
      Self::Note => ('✎', Role::Key),
      Self::Underline => ('‗', Role::Argument),
      Self::Highlight => ('▇', Role::Key),
    })
  }
}

/// `label` within `room` cells around its `matched` bytes, `…` at cut
/// ends; the match's bytes in the result.
pub(super) fn excerpt(
  label: &str,
  matched: Range<usize>,
  room: usize,
) -> (String, Range<usize>) {
  let (before, hit, after) = match (
    label.get(..matched.start),
    label.get(matched.clone()),
    label.get(matched.end..),
  ) {
    (Some(before), Some(hit), Some(after)) => (before, hit, after),
    _ => ("", "", label),
  };
  let hit = clipped(hit, room).0;
  let spare = room - hit.width();
  // Half the spare cells each side; a short side lends the rest.
  let before = tail(before, (spare / 2).max(spare - after.width().min(spare)));
  let mut rest = spare.saturating_sub(before.width());
  let start = before.len();
  let end = start + hit.len();
  loop {
    let text = format!("{before}{hit}{}", clipped(after, rest).0);
    // Joined parts can measure wider than each alone.
    if text.width() <= room || rest == 0 {
      return (text, start..end);
    }
    rest -= 1;
  }
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
