use super::InputOwner;
use crate::clues::Clues;
use crate::layout::Screen;
use crate::pointer::{Items, List};
use crate::prompt::{Menu, Prompt};

/// Bottom lists share the space above status.
pub struct Overlay<'a> {
  screen: Screen,
  menu: Option<&'a Menu>,
  clues: &'a Clues,
}

impl<'a> Overlay<'a> {
  pub fn new(
    screen: Screen,
    prompt: Option<&'a Prompt>,
    clues: &'a Clues,
  ) -> Self {
    Self { screen, menu: prompt.and_then(Prompt::menu), clues }
  }

  pub fn owned_list(&self, owner: InputOwner) -> Option<List> {
    match owner {
      InputOwner::Context => Some(List::Menu),
      InputOwner::Region => Some(List::Actions),
      InputOwner::Hints => Some(List::Hints),
      InputOwner::Prompt => self.menu.map(|_| List::Completions),
      _ => self.clues.shown().map(|_| List::Clues),
    }
  }

  pub fn rows(&self) -> u16 {
    self.menu.map_or_else(
      || self.clues.rows(self.screen),
      |menu| {
        u16::try_from(menu.grid(self.screen).visible).unwrap_or(0)
          + menu.footer_rows(self.screen)
      },
    )
  }

  pub fn top(&self) -> u16 {
    self.screen.rows.saturating_sub(1 + self.rows())
  }

  pub fn uncovered(&self, mut page: Screen) -> Screen {
    page.rows = page.rows.saturating_sub(self.rows());
    page
  }

  pub fn completions(&self) -> Items {
    let Some(menu) = self.menu else { return Items::default() };
    menu.grid(self.screen).cells(menu.items().len(), self.top()).collect()
  }

  pub fn clues(&self) -> Items {
    self
      .clues
      .shown()
      .map_or_else(Items::default, |list| list.cells(self.screen, self.top()))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::prompt::PromptKind;
  use vpdf_model::geometry::At;

  fn screen() -> Screen {
    crate::testing::screen::cells(20, 12)
  }

  #[test]
  fn list_follows_input_owner() {
    let mut clues = Clues::default();
    clues.show(vec![("g".into(), "go".into())]);
    let mut prompt = Prompt::new(PromptKind::Command);
    assert_eq!(
      Overlay::new(screen(), Some(&prompt), &clues)
        .owned_list(InputOwner::Prompt),
      None
    );
    prompt.complete(0, vec!["alpha".into(), "beta".into()]);
    let overlay = Overlay::new(screen(), Some(&prompt), &clues);
    for (owner, list) in [
      (InputOwner::Context, List::Menu),
      (InputOwner::Region, List::Actions),
      (InputOwner::Hints, List::Hints),
      (InputOwner::Prompt, List::Completions),
      (InputOwner::Content, List::Clues),
      (InputOwner::Panel, List::Clues),
      (InputOwner::Overview, List::Clues),
      (InputOwner::Peek, List::Clues),
    ] {
      assert_eq!(overlay.owned_list(owner), Some(list));
    }
  }

  #[test]
  fn completion_hits_follow_drawing() {
    let mut prompt = Prompt::new(PromptKind::Command);
    prompt
      .complete(0, (0..15).map(|index| format!("entry {index:02}")).collect());
    prompt.pick(6);
    let clues = Clues::default();
    let overlay = Overlay::new(screen(), Some(&prompt), &clues);
    assert_eq!((overlay.rows(), overlay.top()), (4, 7));
    let cells = overlay.completions();
    let at = At { column: 5, row: 3 };
    let mut output = Vec::new();
    prompt.scene(&mut output, screen(), at, None);
    for row in 7..11u16 {
      let first = usize::from(row - 4);
      assert_eq!(cells.at(0, row), Some(first));
      assert_eq!(cells.at(10, row), Some(first + 8));
      let start = output
        .iter()
        .position(|op| {
          *op
            == vpdf_model::scene::Op::Move {
              row: usize::from(at.row + row),
              column: usize::from(at.column),
            }
        })
        .unwrap();
      let end = output
        .iter()
        .position(|op| {
          *op
            == vpdf_model::scene::Op::Move {
              row: usize::from(at.row + row + 1),
              column: usize::from(at.column),
            }
        })
        .unwrap();
      let drawn = crate::testing::text(&output[start..end]);
      for index in [first, first + 8] {
        assert!(drawn.contains(&format!("entry {index:02}")), "{drawn:?}");
      }
    }
    assert_eq!(cells.at(0, 6), None);
    assert_eq!(cells.at(0, 11), None);
    assert_eq!(cells.at(20, 7), None);
  }

  #[test]
  fn guidance_matches_hit_testing() {
    for rows in [2, 3, 12] {
      let screen = Screen { rows, columns: 80, ..screen() };
      let mut prompt = Prompt::new(PromptKind::Command);
      prompt.replace("help se");
      prompt.complete(5, vec!["search".into(), "selection".into()]);
      prompt.describe(str::to_owned);
      let clues = Clues::default();
      let overlay = Overlay::new(screen, Some(&prompt), &clues);
      let menu = prompt.menu().unwrap();
      assert_eq!(
        overlay.rows(),
        menu.grid(screen).visible as u16 + menu.footer_rows(screen)
      );
      assert_eq!(overlay.completions().at(0, overlay.top()), Some(0));
      let mut output = Vec::new();
      prompt.scene(&mut output, screen, At::default(), None);
      let text = crate::testing::text(&output);
      if rows >= 3 {
        assert!(text.contains("Unselected"), "{text}");
        assert!(text.contains("submit query"), "{text}");
        assert!(text.contains("<Esc> cancel"), "{text}");
        assert_eq!(overlay.completions().at(0, rows - 2), None);
        prompt.pick(0);
        output.clear();
        prompt.scene(&mut output, screen, At::default(), None);
        assert!(crate::testing::text(&output).contains("open"));
      }
    }
  }

  #[test]
  fn completion_space_precedes_clues() {
    let mut clues = Clues::default();
    let prompt = Prompt::new(PromptKind::Command);
    let empty = Overlay::new(screen(), Some(&prompt), &clues);
    assert_eq!((empty.rows(), empty.top()), (0, 11));
    clues.show(vec![("g".into(), "go".into()), ("a-z".into(), "mark".into())]);
    let shown = Overlay::new(screen(), Some(&prompt), &clues);
    assert_eq!((shown.rows(), shown.top()), (2, 9));
    assert_eq!(shown.clues().at(0, 9), Some(0));
    assert_eq!(shown.clues().at(0, 10), None);
    let mut prompt = prompt;
    prompt
      .complete(0, (0..15).map(|index| format!("entry {index:02}")).collect());
    let shown = Overlay::new(screen(), Some(&prompt), &clues);
    assert_eq!((shown.rows(), shown.top()), (4, 7));
    let page = Screen { columns: 12, rows: 3, origin: (4, 5), ..screen() };
    let visible = shown.uncovered(page);
    assert_eq!(
      (visible.columns, visible.rows, visible.origin),
      (12, 0, (4, 5))
    );
  }
}
