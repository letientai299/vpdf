use super::{
  Action, Axis, BINDINGS, D, M, PanelKind, SelectionKind, W, key_for, notation,
  parse, token,
};
use crate::panel::Panel;

pub(crate) fn text() -> String {
  format!(
    include_str!("../help.txt"),
    left = key(Action::Motion(M::Left)),
    down = key(Action::Motion(M::Down)),
    up = key(Action::Motion(M::Up)),
    right = key(Action::Motion(M::Right)),
    word = key(Action::Motion(M::Word)),
    back_word = key(Action::Motion(M::BackWord)),
    end_word = key(Action::Motion(M::EndWord)),
    big_word = key(Action::Motion(M::BigWord)),
    back_big_word = key(Action::Motion(M::BackBigWord)),
    big_end = key(Action::Motion(M::EndBigWord)),
    previous_big_end = key(Action::Motion(M::PreviousBigEnd)),
    previous_sentence = key(Action::Motion(M::PreviousSentence)),
    next_sentence = key(Action::Motion(M::NextSentence)),
    line_start = key(Action::Motion(M::LineStart)),
    first_nonblank = key(Action::Motion(M::FirstNonblank)),
    line_end = key(Action::Motion(M::LineEnd)),
    previous_paragraph = key(Action::Motion(M::PreviousParagraph)),
    next_paragraph = key(Action::Motion(M::NextParagraph)),
    first_page = key(Action::Motion(M::First)),
    last_page = key(Action::Motion(M::Last)),
    next_page = key(Action::Motion(M::NextPage)),
    previous_page = aliases(Action::Motion(M::PreviousPage)),
    previous_section = key(Action::Section(false)),
    next_section = key(Action::Section(true)),
    next_figure = key(Action::Figure(true)),
    previous_figure = key(Action::Figure(false)),
    history_back = key(Action::History(false)),
    history_forward = key(Action::History(true)),
    overview = key(Action::Overview),
    screen_down = key(Action::Screen(1)),
    screen_up = key(Action::Screen(-1)),
    half_down = aliases(Action::Scroll(0.5)),
    half_up = aliases(Action::Scroll(-0.5)),
    page_down = key(Action::Scroll(1.0)),
    page_up = key(Action::Scroll(-1.0)),
    row_down = key(Action::Rows(1)),
    row_up = key(Action::Rows(-1)),
    align_top = key(Action::Align(0.0)),
    align_middle = key(Action::Align(0.5)),
    align_bottom = key(Action::Align(1.0)),
    zoom_in = key(Action::Zoom(1)),
    zoom_out = key(Action::Zoom(-1)),
    fit = key(Action::Fit),
    fit_page = key(Action::FitPage),
    figure_fit = key(Action::FigureFit),
    rotate = key(Action::Rotate),
    crop = key(Action::Crop),
    visual = key(Action::Visual(SelectionKind::Character)),
    visual_line = key(Action::Visual(SelectionKind::Line)),
    swap = key(Action::Swap),
    reselect = key(Action::Reselect),
    yank = key(Action::Yank),
    yank_raw = key(Action::YankRaw),
    yank_path = key(Action::YankPath),
    search_forward = key(Action::Search(false)),
    search_backward = key(Action::Search(true)),
    result_next = key(Action::Result(false)),
    result_previous = key(Action::Result(true)),
    word_search = key(Action::WordSearch),
    region = key(Action::Region),
    region_start = key(Action::RegionStart),
    follow = key(Action::Follow),
    outline = key(Action::Panel(PanelKind::Outline)),
    figures = key(Action::Panel(PanelKind::Figures)),
    results = key(Action::Panel(PanelKind::Results)),
    hints = key(Action::Hints(false)),
    copy_hints = key(Action::Hints(true)),
    links = key(Action::Panel(PanelKind::Links)),
    recent = key(Action::Panel(PanelKind::Recent)),
    annotations = key(Action::Panel(PanelKind::Annotations)),
    local_focus = key(Action::LocalFocus),
    close_panel = key(Action::ClosePanel),
    definition = key(Action::Definition),
    open_url = key(Action::OpenUrl),
    help = key(Action::Panel(PanelKind::Help)),
    quit = key(Action::Quit),
    peek = key(Action::Peek),
    cancel = key(Action::Cancel),
    windows = windows(),
    yank_lines = aliases(Action::YankLine),
    panel_follow = panel(PanelKind::Outline, Action::Follow),
    panel_note = panel(PanelKind::Annotations, Action::Peek),
    panel_keep = panel(PanelKind::Outline, Action::Swap),
    panel_yank = panel(PanelKind::Outline, Action::YankLine),
    panel_close = panel(PanelKind::Outline, Action::ClosePanel),
  )
}

fn windows() -> String {
  format!(
    include_str!("reference/windows.txt"),
    pane_splits = group(&[
      Action::Window(W::Split(Axis::Rows)),
      Action::Window(W::Split(Axis::Columns))
    ]),
    pane_focus = group(&[
      Action::Window(W::Focus(D::Left)),
      Action::Window(W::Focus(D::Down)),
      Action::Window(W::Focus(D::Up)),
      Action::Window(W::Focus(D::Right))
    ]),
    pane_cycle = group(&[
      Action::Window(W::Next),
      Action::Window(W::Previous),
      Action::Window(W::Last)
    ]),
    pane_closing = group(&[Action::Window(W::Close), Action::Window(W::Only)]),
    pane_size = group(&[
      Action::Window(W::Resize(Axis::Rows, 1)),
      Action::Window(W::Resize(Axis::Rows, -1)),
      Action::Window(W::Resize(Axis::Columns, -1)),
      Action::Window(W::Resize(Axis::Columns, 1))
    ]),
    pane_extend = group(&[
      Action::Window(W::Extend(D::Left)),
      Action::Window(W::Extend(D::Right)),
      Action::Window(W::Extend(D::Up)),
      Action::Window(W::Extend(D::Down))
    ]),
    pane_maximize = group(&[
      Action::Window(W::Maximize(Axis::Rows)),
      Action::Window(W::Maximize(Axis::Columns)),
      Action::Window(W::Equalize)
    ]),
    pane_move = group(&[
      Action::Window(W::Move(D::Left)),
      Action::Window(W::Move(D::Down)),
      Action::Window(W::Move(D::Up)),
      Action::Window(W::Move(D::Right))
    ]),
    pane_exchange_tab =
      group(&[Action::Window(W::Exchange), Action::Window(W::ToTab)]),
    pane_rotation = group(&[
      Action::Window(W::Rotate(true)),
      Action::Window(W::Rotate(false))
    ]),
    pane_new = key(Action::Window(W::New)),
    next_tab = key(Action::Tab(true)),
    previous_tab = key(Action::Tab(false)),
    panel_follow = panel(PanelKind::Outline, Action::Follow),
    panel_keep = panel(PanelKind::Outline, Action::Swap),
    recent_follow = panel(PanelKind::Recent, Action::Follow),
  )
}

fn key(action: Action) -> String {
  notation(key_for(action).expect("Missing help binding"))
}

fn aliases(action: Action) -> String {
  BINDINGS
    .iter()
    .filter(|(_, bound, _)| *bound == action)
    .map(|(keys, ..)| notation(keys))
    .collect::<Vec<_>>()
    .join(" ")
}

fn panel(kind: PanelKind, action: Action) -> String {
  let panel = if kind == PanelKind::Recent {
    Panel::recent(Vec::new(), &[])
  } else {
    Panel::new(kind, Vec::new())
  };
  notation(panel.shortcut(action).expect("Missing panel help binding"))
}

fn group(actions: &[Action]) -> String {
  let names: Vec<_> = actions
    .iter()
    .map(|&action| key_for(action).expect("Missing help binding"))
    .collect();
  let events: Vec<_> = names
    .iter()
    .map(|name| parse(name).expect("Invalid help binding"))
    .collect();
  let Some(first) = events.first() else { return String::new() };
  let prefix: String = first
    .iter()
    .enumerate()
    .take(first.len().saturating_sub(1))
    .take_while(|(index, key)| {
      events.iter().all(|keys| keys.get(*index) == Some(*key))
    })
    .filter_map(|(_, key)| token(*key))
    .collect();
  names
    .iter()
    .enumerate()
    .map(|(index, name)| {
      notation(if index == 0 {
        name
      } else {
        name.strip_prefix(&prefix).unwrap_or(name)
      })
    })
    .collect::<Vec<_>>()
    .join("/")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn groups_elide_only_whole_shared_keys() {
    assert_eq!(
      group(&[
        Action::Window(W::Split(Axis::Rows)),
        Action::Window(W::Split(Axis::Columns))
      ]),
      "<C-w>s/v"
    );
    assert_eq!(
      group(&[Action::History(false), Action::History(true)]),
      "<C-o>/<C-i>"
    );
    assert_eq!(
      group(&[Action::Window(W::Split(Axis::Rows)), Action::Quit]),
      "<C-w>s/q"
    );
    assert_eq!(group(&[Action::Window(W::New)]), "<C-w>n");
    assert_eq!(group(&[]), "");
  }

  #[test]
  fn reference_preserves_panel_and_operator_meanings() {
    let help = text();
    for line in [
      "P q: close focused panel",
      "gq: close sidebar from either focus",
      "o: swap selection ends",
      "P o: follow; keep panel focus",
      "P yy: yank row",
      "yy Y: join wrapped lines; rejoin hyphenated words",
      "V / y + i/a: inner / around objects",
      "( ) [ ] { } < >: paired brackets",
    ] {
      assert!(help.lines().any(|actual| actual == line), "{line}");
    }
  }
}
