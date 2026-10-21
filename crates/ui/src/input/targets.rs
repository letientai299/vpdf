use super::{InputOwner, Overlay};
use crate::context::ContextMenu;
use crate::hints::Hints;
use crate::panel::Panel;
use crate::pointer::{Hit, Items, List, Region};
use crate::preview::Preview;
use crate::region::Region as Capture;
use crate::text::PageId;
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

/// Borrowed components for pointer routing.
pub struct Targets<'a> {
  pub panel: Option<&'a Panel>,
  pub hints: Option<&'a Hints>,
  pub preview: Option<&'a Preview>,
  pub context: Option<&'a ContextMenu>,
  pub capture: Option<&'a Capture>,
  pub bottom: Overlay<'a>,
  pub owner: InputOwner,
}

impl Targets<'_> {
  pub fn owned_list(&self) -> Option<List> {
    self.bottom.owned_list(self.owner)
  }

  pub fn items(&self, list: List) -> Items {
    match list {
      List::Menu => self.context.map(ContextMenu::items),
      List::Actions => self.capture.map(Capture::items),
      List::Hints => self.hints.map(Hints::hit_items),
      List::Clues => Some(self.bottom.clues()),
      List::Completions => Some(self.bottom.completions()),
    }
    .unwrap_or_default()
  }

  pub fn hovered(&self, hit: Hit) -> Option<(List, usize)> {
    let list = self.owned_list()?;
    hit.item(list).map(|index| (list, index))
  }

  /// Mouse coordinates are relative to the pane.
  pub fn pressed(&self, mouse: MouseEvent) -> Option<(List, usize)> {
    if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
      return None;
    }
    let list = self.owned_list()?;
    if list == List::Actions && mouse.modifiers.contains(KeyModifiers::ALT) {
      return None;
    }
    self.items(list).at(mouse.column, mouse.row).map(|index| (list, index))
  }

  /// None requests the page's document hit.
  pub fn hit(
    &self,
    region: Region,
    (column, row): (u16, u16),
    scrollbar: impl FnOnce() -> bool,
    thumbnail: impl FnOnce() -> Option<PageId>,
    status: impl FnOnce() -> Option<crate::chrome::Item>,
  ) -> Option<Hit> {
    Some(match region {
      Region::Page => {
        if scrollbar() {
          return Some(Hit::Scrollbar);
        }
        let index =
          self.hints.and_then(|hints| hints.hit_items().at(column, row));
        return index.map(|index| Hit::Item(List::Hints, Some(index)));
      }
      Region::Panel => {
        let Some(panel) = self.panel else { return Some(Hit::Nothing) };
        if let Some(index) = panel.row_at(column, row) {
          if panel.rows()[index].follows() {
            Hit::Row(index)
          } else {
            Hit::Label(index)
          }
        } else if panel.shelf().is_some_and(|shelf| shelf.covers(column)) {
          Hit::Card(row)
        } else {
          Hit::Nothing
        }
      }
      Region::Overview => thumbnail().map_or(Hit::Nothing, Hit::Thumbnail),
      Region::Menu | Region::Actions => {
        let list =
          if region == Region::Menu { List::Menu } else { List::Actions };
        Hit::Item(list, self.items(list).at(column, row))
      }
      Region::Preview => {
        Hit::Preview { follows: self.preview.is_some_and(Preview::follows) }
      }
      Region::Overlay => {
        let list = self
          .owned_list()
          .filter(|list| matches!(list, List::Clues | List::Completions));
        let index = list.and_then(|list| self.items(list).at(column, row));
        list.map_or(Hit::Nothing, |list| Hit::Item(list, index))
      }
      Region::Status => status().map_or(Hit::Nothing, |_| Hit::Chrome),
      Region::Edge => Hit::Nothing,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::clues::Clues;
  use crate::layout::Screen;
  use std::cell::Cell;

  fn targets(clues: &Clues) -> Targets<'_> {
    let screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    Targets {
      panel: None,
      hints: None,
      preview: None,
      context: None,
      capture: None,
      bottom: Overlay::new(screen, None, clues),
      owner: InputOwner::Content,
    }
  }

  #[test]
  fn panel_hits_require_destination() -> crate::Result<()> {
    use crate::keys::PanelKind;
    use crate::panel::{Destination, Row};
    use vpdf_model::style::Shape;
    let clues = Clues::default();
    let mut panel = Panel::new(
      PanelKind::Links,
      vec![
        Row::parse("No links"),
        Row::new(
          "Open file".into(),
          String::new(),
          Destination::File("book.pdf".into()),
        ),
      ],
    );
    panel.list(
      &mut Vec::new(),
      vpdf_terminal::terminal::At::default(),
      40,
      6,
    )?;
    let mut targets = targets(&clues);
    targets.panel = Some(&panel);
    for (row, hit, shape) in
      [(1, Hit::Label(0), Shape::Default), (2, Hit::Row(1), Shape::Pointer)]
    {
      let actual =
        targets.hit(Region::Panel, (1, row), || false, || None, || None);
      assert_eq!(actual, Some(hit));
      assert_eq!(hit.shape(), shape);
    }
    Ok(())
  }

  #[test]
  fn menu_capture_precedes_prompt() {
    use super::super::mouse::Surface;
    use crate::pointer::Wheel;
    let clues = Clues::default();
    let menu = ContextMenu::new(vec![], (0, 0), (0, 80), 23, false);
    let mut targets = targets(&clues);
    targets.context = Some(&menu);
    targets.owner = InputOwner::Prompt;
    for kind in [
      MouseEventKind::Drag(MouseButton::Right),
      MouseEventKind::Up(MouseButton::Right),
    ] {
      assert_eq!(targets.surface(kind, Region::Page), Surface::MenuDrag);
    }
    assert_eq!(
      targets.surface(MouseEventKind::ScrollDown, Region::Menu),
      Surface::Ignore
    );
    targets.owner = InputOwner::Context;
    for (region, expected) in [
      (Region::Menu, Surface::MenuWheel),
      (Region::Status, Surface::MenuWheel),
      (Region::Overlay, Surface::Clues(Wheel::Down)),
    ] {
      assert_eq!(targets.surface(MouseEventKind::ScrollDown, region), expected);
      assert_eq!(
        targets.surface(MouseEventKind::Moved, region),
        Surface::Ignore
      );
    }
    let capture = Capture::open(crate::region::Snapshot {
      id: crate::region::CaptureId::new(1),
      page: PageId(0),
      bounds: crate::text::Bounds::default(),
    });
    targets.capture = Some(&capture);
    assert_eq!(
      targets.surface(MouseEventKind::ScrollDown, Region::Page),
      Surface::Ignore
    );
    assert_eq!(
      targets.surface(MouseEventKind::Up(MouseButton::Right), Region::Page),
      Surface::MenuDrag
    );
  }

  #[test]
  fn lists_require_their_owner() {
    use crate::context::{Command, Item};
    let clues = Clues::default();
    let menu = ContextMenu::new(
      vec![Item {
        label: "Follow".into(),
        key: "Enter".into(),
        command: Command::Follow(vpdf_model::document::Target::Page(PageId(0))),
      }],
      (4, 5),
      (0, 80),
      23,
      false,
    );
    let mut targets = targets(&clues);
    targets.context = Some(&menu);
    let mouse = MouseEvent {
      kind: MouseEventKind::Down(MouseButton::Left),
      column: 4,
      row: 5,
      modifiers: KeyModifiers::NONE,
    };
    let hit = Hit::Item(List::Menu, Some(0));
    assert_eq!(targets.hovered(hit), None);
    assert_eq!(targets.pressed(mouse), None);
    targets.owner = InputOwner::Context;
    assert_eq!(targets.hovered(hit), Some((List::Menu, 0)));
    assert_eq!(targets.hovered(Hit::Item(List::Hints, Some(0))), None);
    assert_eq!(targets.pressed(mouse), Some((List::Menu, 0)));
    assert_eq!(
      targets.pressed(MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        ..mouse
      }),
      None
    );
    assert_eq!(targets.pressed(MouseEvent { column: 79, ..mouse }), None);
    assert_eq!(
      targets.pressed(MouseEvent { modifiers: KeyModifiers::ALT, ..mouse }),
      Some((List::Menu, 0))
    );
  }

  #[test]
  fn action_bar_preserves_alt() -> crate::Result<()> {
    use crate::layout::Layout;
    use crate::paint::Frame;
    use crate::region::{Canvas, CaptureId, Snapshot};
    use crate::text::Bounds;
    use vpdf_terminal::{graphics::Graphics, terminal::At};
    let screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let bounds = Bounds { x0: 0.0, y0: 0.0, x1: 200.0, y1: 200.0 };
    let layout = Layout::new([bounds], screen);
    let canvas = Canvas {
      layout: &layout,
      scroll: 0.0,
      frame: Frame { screen, at: At::default(), left: 0, text: true },
    };
    let mut capture = Capture::open(Snapshot {
      id: CaptureId::new(1),
      page: PageId(0),
      bounds,
    });
    canvas.draw(
      Some(&mut capture),
      None,
      &mut Graphics::default(),
      &mut Vec::new(),
    )?;
    let clues = Clues::default();
    let mut targets = targets(&clues);
    targets.capture = Some(&capture);
    targets.owner = InputOwner::Region;
    let area = capture.cells().unwrap();
    let column = (area.column..area.column + area.columns)
      .find(|&column| capture.items().at(column, area.row).is_some())
      .unwrap();
    let mouse = MouseEvent {
      kind: MouseEventKind::Down(MouseButton::Left),
      column,
      row: area.row,
      modifiers: KeyModifiers::NONE,
    };
    assert!(matches!(targets.pressed(mouse), Some((List::Actions, _))));
    assert_eq!(
      targets.pressed(MouseEvent { modifiers: KeyModifiers::ALT, ..mouse }),
      None
    );
    Ok(())
  }

  #[test]
  fn surface_queries_stay_lazy() {
    let clues = Clues::default();
    let targets = targets(&clues);
    for (region, expected, queries) in [
      (Region::Page, None, 1),
      (Region::Panel, Some(Hit::Nothing), 0),
      (Region::Overview, Some(Hit::Thumbnail(PageId(2))), 2),
      (Region::Menu, Some(Hit::Item(List::Menu, None)), 0),
      (Region::Actions, Some(Hit::Item(List::Actions, None)), 0),
      (Region::Preview, Some(Hit::Preview { follows: false }), 0),
      (Region::Overlay, Some(Hit::Nothing), 0),
      (Region::Status, Some(Hit::Chrome), 4),
      (Region::Edge, Some(Hit::Nothing), 0),
    ] {
      let called = Cell::new(0);
      let hit = targets.hit(
        region,
        (4, 5),
        || {
          called.set(called.get() | 1);
          false
        },
        || {
          called.set(called.get() | 2);
          Some(PageId(2))
        },
        || {
          called.set(called.get() | 4);
          Some(crate::chrome::Item::Pages)
        },
      );
      assert_eq!(hit, expected, "{region:?}");
      assert_eq!(called.get(), queries, "{region:?}");
    }
  }

  #[test]
  fn page_overlays_precede_text() -> crate::Result<()> {
    let clues = Clues::default();
    let hints = Hints::new(
      vec![(((4, 5), 8), vpdf_model::document::Target::Page(PageId(0)))],
      false,
      0,
      80,
    )?;
    let mut targets = targets(&clues);
    targets.hints = Some(&hints);
    for (bar, expected) in
      [(true, Hit::Scrollbar), (false, Hit::Item(List::Hints, Some(0)))]
    {
      assert_eq!(
        targets.hit(
          Region::Page,
          (3, 5),
          || bar,
          || panic!("Unexpected thumbnail"),
          || panic!("Unexpected status")
        ),
        Some(expected)
      );
    }
    assert_eq!(
      targets.hit(
        Region::Page,
        (40, 5),
        || false,
        || panic!("Unexpected thumbnail"),
        || panic!("Unexpected status")
      ),
      None
    );
    Ok(())
  }
}
