use super::{Destination, Row, RowKind, Style, UnicodeWidthStr};
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use vpdf_model::scene::{Draw, Op};
use vpdf_model::style::Role;
use vpdf_model::text::{clipped, tail};

impl Row {
  /// The label after its caption key, as in `Fig 2 Overview`, for
  /// copies and hover; other keys only locate the row.
  pub fn full_label(&self) -> String {
    let label = self.label.trim();
    let caption = matches!(self.kind, RowKind::Figure | RowKind::Table);
    if self.key.is_empty() || !caption {
      return label.to_owned();
    }
    format!("{} {label}", self.key)
  }

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

impl Row {
  /// `# Section` headings over their lines; lines before the first
  /// heading form a header block: a title, then muted facts.
  pub fn sections(text: &str) -> Vec<Self> {
    let mut sections = false;
    let mut rows: Vec<Self> = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
      let row = Self::parse(line);
      sections |= row.style == Style::Heading;
      rows.push(if sections {
        Self { depth: usize::from(row.style != Style::Heading), ..row }
      } else if rows.is_empty() {
        Self { style: Style::Plain, ..row }
      } else {
        Self { style: Style::Muted, ..row }
      });
    }
    rows
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
          marked(scene, &key[start..end], query, base, plain.then_some(token));
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
        let directory = matches!(self.destination, Destination::Directory(_));
        let text = if directory {
          tail(&self.label, room)
        } else {
          vpdf_model::text::clean(&self.label, room)
        };
        let role = if directory {
          plain.then_some(Role::Muted)
        } else {
          Some(Role::Heading)
        };
        marked(scene, &text, query, base, role);
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

/// Muted on unselected rows; else `base`.
pub(super) fn quiet(base: Role) -> Role {
  if accented(Some(base)) { Role::Muted } else { base }
}

/// Row glyphs in ASCII, for fonts or terminals without the symbols.
static ASCII: AtomicBool = AtomicBool::new(false);

pub fn set_ascii_glyphs(ascii: bool) {
  ASCII.store(ascii, Ordering::Relaxed);
}

impl RowKind {
  pub fn glyph(self) -> Option<(char, Role)> {
    self.glyph_in(ASCII.load(Ordering::Relaxed))
  }

  pub(super) fn glyph_in(self, ascii: bool) -> Option<(char, Role)> {
    let (symbol, plain, role) = match self {
      Self::Plain => return None,
      Self::Figure => ('▣', 'F', Role::Heading),
      Self::Section => ('§', '#', Role::Heading),
      Self::Table => ('▤', 'T', Role::Mode),
      Self::Mention => ('¶', '~', Role::Muted),
      Self::Search => ('⌕', '/', Role::Key),
      Self::Link => ('↗', '>', Role::Mode),
      Self::Note => ('✎', '*', Role::Key),
      Self::Underline => ('‗', '_', Role::Argument),
      Self::Highlight => ('▇', '=', Role::Key),
    };
    Some((if ascii { plain } else { symbol }, role))
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
