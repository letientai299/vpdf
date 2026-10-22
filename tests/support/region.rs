use vpdf_model::geometry::{At, Screen};
use vpdf_model::text::{Bounds, PageId};
use vpdf_ui::layout::Layout;
use vpdf_ui::paint::Frame;
use vpdf_ui::region::{Canvas, CaptureId, Snapshot};

pub fn snapshot(id: u64) -> Snapshot {
  Snapshot {
    id: CaptureId::new(id),
    page: PageId(0),
    bounds: Bounds { x0: 10.0, y0: 20.0, x1: 60.0, y1: 80.0 },
  }
}

pub fn layout() -> Layout {
  let screen = Screen {
    columns: 20,
    rows: 10,
    cell_width: 8,
    cell_height: 16,
    origin: (2, 1),
  };
  Layout::new([Bounds { x0: 0.0, y0: 0.0, x1: 160.0, y1: 160.0 }], screen)
}

pub fn canvas(layout: &Layout) -> Canvas<'_> {
  Canvas {
    layout,
    scroll: 0.0,
    frame: Frame {
      screen: layout.screen(),
      at: At { column: 5, row: 1 },
      left: 3,
      text: false,
    },
  }
}

pub fn sweep() -> (PageId, Bounds) {
  (PageId(0), Bounds { x0: 12.5, y0: 24.5, x1: 63.2, y1: 81.7 })
}
