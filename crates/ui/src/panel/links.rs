use super::layout::Layout;
use super::{Destination, Row, Target};
impl Layout {
  pub fn links(rows: &[Row], columns: usize, selected: usize) -> Self {
    let mut layout = Self { columns, lines: Vec::new(), starts: Vec::new() };
    for (source, row) in rows.iter().enumerate() {
      layout.starts.push(layout.lines.len());
      layout.label(source, &row.label, &row.detail, selected);
      if let Destination::Link(Target::Uri(uri)) = &row.destination
        && *uri != row.label
      {
        layout.secondary(source, uri, selected);
      }
    }
    layout.starts.push(layout.lines.len());
    layout
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    Result,
    keys::{Action, Input, PanelKind},
    panel::{Panel, Style},
  };
  use crossterm::event::{
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
  };
  use unicode_width::UnicodeWidthStr;
  use vpdf_reader::motion::Motion;
  use vpdf_terminal::terminal;
  fn row(label: &str, uri: &str) -> Row {
    Row::new(
      label.into(),
      String::new(),
      Destination::Link(Target::Uri(uri.into())),
    )
  }
  #[test]
  fn unselected_long_labels_clip() {
    let long = "A label far wider than the panel";
    let rows = [row(long, long), row(long, long), row("Short", "Short")];
    let layout = Layout::links(&rows, 20, 0);
    // (source) -> its label line clipped
    let cases = [(0, false), (1, true), (2, false)];
    for (source, want) in cases {
      let label = &layout.lines[layout.starts[source]];
      assert_eq!(label.clipped, want, "row {source}");
    }
    assert!(layout.lines.iter().filter(|line| line.source == 0).count() > 1);
  }
  #[test]
  fn selected_urls_expand() {
    let uri = "https://example.com/long/path?query=needle";
    let rows = [row("Project page", uri), row(uri, uri)];
    for columns in [4, 12, 30, 50] {
      let layout = Layout::links(&rows, columns, 0);
      let secondary: String = layout
        .lines
        .iter()
        .filter(|line| line.source == 0 && line.row.style == Style::Muted)
        .map(|line| line.row.label.trim_start())
        .collect();
      assert_eq!(secondary, uri);
      assert!(layout.lines.iter().all(|line| line.row.label.width()
        + line.row.detail.width()
        <= columns));
      assert_eq!(
        layout.lines.iter().filter(|line| line.source == 1).count(),
        1
      );
      let layout = Layout::links(&rows, columns, 1);
      let raw: String = layout
        .lines
        .iter()
        .filter(|line| line.source == 1)
        .map(|line| line.row.label.as_str())
        .collect();
      assert_eq!(raw, uri);
    }
  }
  #[test]
  fn expanded_links_keep_targets() -> Result<()> {
    let uri = "https://example.com/long/path?query=needle";
    let mut panel = Panel::new(
      PanelKind::Links,
      vec![row("Project page", uri), row("Other", "https://example.org")],
    );
    panel.list(&mut Vec::new(), terminal::At::default(), 24, 20)?;
    let press = |line| MouseEvent {
      kind: MouseEventKind::Down(MouseButton::Left),
      column: 4,
      row: line,
      modifiers: KeyModifiers::NONE,
    };
    assert_eq!(panel.pointer(press(2)), Some(0));
    panel.navigate(Input::plain(Action::Motion(Motion::Down)), 20);
    panel.list(&mut Vec::new(), terminal::At::default(), 24, 20)?;
    assert_eq!(panel.selected, 1);
    panel.begin_search(false);
    panel.preview("needle");
    assert_eq!(panel.selected, 0);
    assert_eq!(panel.count(), Some((1, 1)));
    panel.list(&mut Vec::new(), terminal::At::default(), 12, 20)?;
    assert_eq!(panel.pointer(press(3)), Some(0));
    Ok(())
  }
  #[test]
  fn wheel_scrolls_expanded_links() -> Result<()> {
    let uri = format!("https://example.com/{}", "long-path/".repeat(20));
    let mut panel = Panel::new(
      PanelKind::Links,
      vec![row(&uri, &uri), row("Other", "https://example.org")],
    );
    panel.list(&mut Vec::new(), terminal::At::default(), 20, 6)?;
    panel.pointer(MouseEvent {
      kind: MouseEventKind::ScrollDown,
      column: 4,
      row: 2,
      modifiers: KeyModifiers::NONE,
    });
    panel.list(&mut Vec::new(), terminal::At::default(), 20, 6)?;
    assert_eq!((panel.selected, panel.top), (0, 3));
    panel.navigate(Input::plain(Action::Motion(Motion::Down)), 6);
    let mut out = Vec::new();
    panel.list(&mut out, terminal::At::default(), 20, 6)?;
    assert_eq!(panel.selected, 1);
    assert!(String::from_utf8(out)?.contains("Other"));
    Ok(())
  }
}
