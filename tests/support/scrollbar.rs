use crate::fixtures::screen;
use vpdf_model::input::MouseEventKind;
use vpdf_model::text::Bounds;
use vpdf_ui::layout::Layout;
use vpdf_ui::paint::Frame;
use vpdf_ui::pointer::Route;
use vpdf_ui::scrollbar::View;

pub fn layout() -> Layout {
  let bounds = Bounds { x0: 0.0, y0: 0.0, x1: 960.0, y1: 1000.0 };
  Layout::new([bounds; 3], screen::cells(120, 40))
}

pub fn view(layout: &Layout, scroll: f32) -> View<'_> {
  View {
    layout,
    scroll,
    available: true,
    hovered: true,
    frame: Frame {
      screen: layout.screen(),
      left: 0,
      text: false,
      at: vpdf_model::geometry::At::default(),
    },
  }
}

pub fn route(kind: MouseEventKind, row: u16) -> Route {
  Route {
    pane: 1,
    region: vpdf_ui::pointer::Region::Page,
    focus: true,
    clicks: 1,
    pixel: None,
    mouse: vpdf_model::input::MouseEvent {
      kind,
      row,
      column: 119,
      modifiers: vpdf_model::input::KeyModifiers::NONE,
    },
  }
}

pub fn scrub(
  layout: &Layout,
  now: std::time::Instant,
) -> (
  vpdf_ui::scrollbar::Bar,
  vpdf_model::text::PageId,
  vpdf_model::scene::Scene<'static>,
) {
  use vpdf_model::scene::Scene;
  use vpdf_reader::reader::{Location, Reader};
  use vpdf_ui::find::Search;
  use vpdf_ui::scrollbar::{Bar, Input, Ticks};
  let view = view(layout, 100.0);
  let ticks = Ticks { outline: &[], search: &Search::default() };
  let mut bar = Bar::default();
  let Input::Press(press) = bar.input(
    &view,
    &route(MouseEventKind::Down(vpdf_model::input::MouseButton::Left), 5),
  ) else {
    panic!("Expected scrollbar press")
  };
  press.apply(&mut bar, Location::default(), &mut Reader::default());
  let mut scene = Scene::new();
  let label = bar.scene(view, ticks, now, &mut scene).unwrap();
  let page = label.page();
  label.scene("Section", &mut scene);
  (bar, page, scene)
}
