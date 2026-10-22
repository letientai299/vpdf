use anyhow::Result;
use std::io::Write;
use std::time::Instant;
use vpdf_model::geometry::{At, Screen};
use vpdf_model::style::Role;
use vpdf_model::text::{Bounds, PageId};
use vpdf_reader::reader::TextSource;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::overlays::Overlays;
use vpdf_ui::paint::{Frame, Marks};
use vpdf_ui::panel::Panel;
use vpdf_ui::panel::recent::Shelf;
use vpdf_ui::source::{Markup, Pages, Raster};
pub mod clues_listdraw {
  use super::{At, Result, Screen, Write};
  use vpdf_ui::clues::List;
  pub trait ListDraw {
    fn draw(
      &self,
      out: &mut impl Write,
      bottom: (At, u16),
      screen: Screen,
      pointed: Option<usize>,
    ) -> Result<()>;
  }
  impl ListDraw for List {
    fn draw(
      &self,
      out: &mut impl Write,
      bottom: (At, u16),
      screen: Screen,
      pointed: Option<usize>,
    ) -> Result<()> {
      let mut painter = vpdf_terminal::painter::Painter::new(out);
      self.scene(&mut painter, bottom, screen, pointed);
      painter.finish()?;
      Ok(())
    }
  }
}
pub use clues_listdraw::ListDraw;

pub mod context_contextdraw {
  use super::{Frame, Graphics, Result, Write};
  use vpdf_ui::context::ContextMenu;
  pub trait ContextDraw {
    fn draw(
      &self,
      graphics: &mut Graphics,
      out: &mut impl Write,
      frame: Frame,
    ) -> Result<()>;
  }
  impl<C> ContextDraw for ContextMenu<C> {
    fn draw(
      &self,
      graphics: &mut Graphics,
      out: &mut impl Write,
      frame: Frame,
    ) -> Result<()> {
      let mut scene =
        vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
      self.scene(&mut scene, frame);
      scene.finish()?;
      Ok(())
    }
  }
}
pub use context_contextdraw::ContextDraw;

pub mod split_nodedraw {
  use super::{Result, Write};
  use vpdf_ui::split::{Node, Rect};
  pub trait NodeDraw {
    fn draw_dividers(&self, out: &mut impl Write, area: Rect) -> Result<()>;
  }
  impl NodeDraw for Node {
    fn draw_dividers(&self, out: &mut impl Write, area: Rect) -> Result<()> {
      let mut scene = vpdf_terminal::painter::Painter::new(out);
      self.scene_dividers(&mut scene, area);
      scene.finish()?;
      Ok(())
    }
  }
}
pub use split_nodedraw::NodeDraw;

pub mod text_view_textdraw {
  use super::{Marks, Pages, Result, TextSource, Write};
  use vpdf_ui::text_view::Pane;
  pub trait TextDraw<S: TextSource + Pages> {
    fn draw(&mut self, out: &mut impl Write, paint: &Marks<'_>) -> Result<()>;
  }
  impl<S: TextSource + Pages> TextDraw<S> for Pane<'_, S> {
    fn draw(&mut self, out: &mut impl Write, paint: &Marks<'_>) -> Result<()> {
      let mut painter = vpdf_terminal::painter::Painter::new(out);
      self.scene(&mut painter, paint)?;
      painter.finish()?;
      Ok(())
    }
  }
}
pub use text_view_textdraw::TextDraw;

pub mod overview_overviewdraw {
  use super::{Graphics, Pages, Result, Write};
  use vpdf_ui::layout::tiles::Visible;
  use vpdf_ui::overview::Frame;
  use vpdf_ui::overview::Overview;
  pub trait OverviewDraw {
    fn draw(
      &mut self,
      source: &impl Pages,
      frame: Frame,
      graphics: &mut Graphics,
      out: &mut impl Write,
    ) -> Result<Visible>;
  }
  impl OverviewDraw for Overview {
    fn draw(
      &mut self,
      source: &impl Pages,
      frame: Frame,
      graphics: &mut Graphics,
      out: &mut impl Write,
    ) -> Result<Visible> {
      let mut scene =
        vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
      let visible = self.scene(source, frame, &mut scene);
      scene.finish()?;
      visible
    }
  }
}
pub use overview_overviewdraw::OverviewDraw;

pub mod perf_perfdraw {
  use super::{Result, Screen, Write};
  use vpdf_ui::perf::Overlay;
  pub trait PerfDraw {
    fn draw(
      &mut self,
      out: &mut impl Write,
      screen: Screen,
      force: bool,
    ) -> Result<bool>;
  }
  impl PerfDraw for Overlay {
    fn draw(
      &mut self,
      out: &mut impl Write,
      screen: Screen,
      force: bool,
    ) -> Result<bool> {
      let mut scene = vpdf_terminal::painter::Painter::new(out);
      let changed = self.scene(&mut scene, screen, force);
      scene.finish()?;
      Ok(changed)
    }
  }
}
pub use perf_perfdraw::PerfDraw;

pub mod hints_hintsdraw {
  use super::{Frame, Graphics, Result, Write};
  use vpdf_ui::hints::Hints;
  pub trait HintsDraw {
    fn draw(
      &self,
      graphics: &mut Graphics,
      out: &mut impl Write,
      frame: Frame,
    ) -> Result<()>;
  }
  impl HintsDraw for Hints {
    fn draw(
      &self,
      graphics: &mut Graphics,
      out: &mut impl Write,
      frame: Frame,
    ) -> Result<()> {
      let mut scene =
        vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
      self.scene(&mut scene, frame);
      scene.finish()?;
      Ok(())
    }
  }
}
pub use hints_hintsdraw::HintsDraw;

pub mod chrome_noticedraw {
  use super::{Result, Write};
  use vpdf_ui::chrome::Notice;
  pub trait NoticeDraw {
    fn draw(&self, out: &mut impl Write) -> Result<()>;
  }
  impl NoticeDraw for Notice {
    fn draw(&self, out: &mut impl Write) -> Result<()> {
      let mut painter = vpdf_terminal::painter::Painter::new(out);
      self.scene(&mut painter);
      painter.finish()?;
      Ok(())
    }
  }
}
pub use chrome_noticedraw::NoticeDraw;

pub mod chrome_chromedraw {
  use super::{At, Result, Write};
  use vpdf_ui::chrome::Bar;
  pub trait ChromeDraw {
    fn draw(
      &self,
      out: &mut impl Write,
      at: At,
      row: u16,
      focused: bool,
      detail: Option<&str>,
    ) -> Result<()>;
  }
  impl ChromeDraw for Bar<'_> {
    fn draw(
      &self,
      out: &mut impl Write,
      at: At,
      row: u16,
      focused: bool,
      detail: Option<&str>,
    ) -> Result<()> {
      let mut painter = vpdf_terminal::painter::Painter::new(out);
      self.scene(&mut painter, at, row, focused, detail)?;
      painter.finish()?;
      Ok(())
    }
  }
}
pub use chrome_chromedraw::ChromeDraw;

pub mod prompt_promptdraw {
  use super::{At, Result, Screen, Write};
  use vpdf_ui::prompt::Prompt;
  pub trait PromptDraw {
    fn draw(
      &self,
      out: &mut impl Write,
      screen: Screen,
      at: At,
      pointed: Option<usize>,
    ) -> Result<()>;
  }
  impl PromptDraw for Prompt {
    fn draw(
      &self,
      out: &mut impl Write,
      screen: Screen,
      at: At,
      pointed: Option<usize>,
    ) -> Result<()> {
      let mut scene = vpdf_terminal::painter::Painter::new(out);
      self.scene(&mut scene, screen, at, pointed);
      scene.finish()?;
      Ok(())
    }
  }
}
pub use prompt_promptdraw::PromptDraw;

pub mod region_regiondraw {
  use super::{Bounds, Graphics, PageId, Result, Write};
  use vpdf_ui::region::{Canvas, Region};
  pub trait RegionDraw {
    fn draw(
      &self,
      region: Option<&mut Region>,
      sweep: Option<(PageId, Bounds)>,
      graphics: &mut Graphics,
      out: &mut impl Write,
    ) -> Result<()>;
  }
  impl RegionDraw for Canvas<'_> {
    fn draw(
      &self,
      region: Option<&mut Region>,
      sweep: Option<(PageId, Bounds)>,
      graphics: &mut Graphics,
      out: &mut impl Write,
    ) -> Result<()> {
      let snap_to_cells = graphics.crop_quirk();
      let mut scene =
        vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
      self.scene(region, sweep, snap_to_cells, &mut scene);
      scene.finish()?;
      Ok(())
    }
  }
}
pub use region_regiondraw::RegionDraw;

pub mod page_pagedraw {
  use super::{Graphics, Marks, Pages, Result, TextSource, Write};
  use vpdf_ui::page::Canvas;
  pub trait PageDraw {
    fn draw(
      &self,
      source: &mut (impl TextSource + Pages),
      marks: &Marks<'_>,
      graphics: &mut Graphics,
      out: &mut impl Write,
    ) -> Result<()>;
  }
  impl PageDraw for Canvas<'_> {
    fn draw(
      &self,
      source: &mut (impl TextSource + Pages),
      marks: &Marks<'_>,
      graphics: &mut Graphics,
      out: &mut impl Write,
    ) -> Result<()> {
      let mut painter =
        vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
      let drawn = self.scene(source, marks, &mut painter);
      painter.finish()?;
      drawn
    }
  }
}
pub use page_pagedraw::PageDraw;

pub mod home_homedraw {
  use super::{Graphics, Result, Shelf, Write};
  use vpdf_ui::home::Home;
  pub trait HomeDraw {
    fn draw(
      &mut self,
      out: &mut impl Write,
      graphics: &mut Graphics,
      now: u64,
      request: impl FnOnce(&mut Shelf),
    ) -> Result<()>;
  }
  impl HomeDraw for Home {
    fn draw(
      &mut self,
      out: &mut impl Write,
      graphics: &mut Graphics,
      now: u64,
      request: impl FnOnce(&mut Shelf),
    ) -> Result<()> {
      let mut painter =
        vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
      let result = self.scene(&mut painter, now, request);
      painter.finish()?;
      result
    }
  }
}
pub use home_homedraw::HomeDraw;

pub mod scrollbar_scrollbardraw {
  use super::{Graphics, Instant, Result, Write};
  use vpdf_ui::scrollbar::{Bar, Label, Ticks, View};
  pub trait ScrollbarDraw {
    fn draw(
      &mut self,
      view: View<'_>,
      ticks: Ticks<'_>,
      now: Instant,
      graphics: &mut Graphics,
      out: &mut impl Write,
    ) -> Result<Option<Label>>;
  }
  impl ScrollbarDraw for Bar {
    fn draw(
      &mut self,
      view: View<'_>,
      ticks: Ticks<'_>,
      now: Instant,
      graphics: &mut Graphics,
      out: &mut impl Write,
    ) -> Result<Option<Label>> {
      let mut scene =
        vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
      let label = self.scene(view, ticks, now, &mut scene);
      scene.finish()?;
      Ok(label)
    }
  }
}
pub use scrollbar_scrollbardraw::ScrollbarDraw;

pub mod scrollbar_labeldraw {
  use super::{Graphics, Result, Write};
  use vpdf_ui::scrollbar::Label;
  pub trait LabelDraw {
    fn draw(
      self,
      text: &str,
      graphics: &mut Graphics,
      out: &mut impl Write,
    ) -> Result<()>;
  }
  impl LabelDraw for Label {
    fn draw(
      self,
      text: &str,
      graphics: &mut Graphics,
      out: &mut impl Write,
    ) -> Result<()> {
      let mut scene =
        vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
      self.scene(text, &mut scene);
      scene.finish()?;
      Ok(())
    }
  }
}
pub use scrollbar_labeldraw::LabelDraw;

pub mod layout_tiles_tilesdraw {
  use super::{Graphics, Result, Write};
  use vpdf_ui::layout::tiles::Frame;
  use vpdf_ui::layout::tiles::{Tiles, Update, View};
  pub trait TilesDraw {
    fn draw(
      &mut self,
      view: View<'_>,
      frame: Frame,
      graphics: &mut Graphics,
      out: &mut impl Write,
    ) -> Result<Update>;
  }
  impl TilesDraw for Tiles {
    fn draw(
      &mut self,
      view: View<'_>,
      frame: Frame,
      graphics: &mut Graphics,
      out: &mut impl Write,
    ) -> Result<Update> {
      let mut scene =
        vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
      let update = self.scene(view, frame, &mut scene);
      scene.finish()?;
      update
    }
  }
}
pub use layout_tiles_tilesdraw::TilesDraw;

pub mod preview_view_previewdraw {
  use super::{Graphics, Pages, Result, Write};
  use vpdf_ui::preview::Frame;
  use vpdf_ui::preview::Preview;
  pub trait PreviewDraw {
    fn draw(
      &mut self,
      graphics: &mut Graphics,
      out: &mut impl Write,
      input: Frame<'_>,
      source: &impl Pages,
    ) -> Result<()>;
  }
  impl PreviewDraw for Preview {
    fn draw(
      &mut self,
      graphics: &mut Graphics,
      out: &mut impl Write,
      input: Frame<'_>,
      source: &impl Pages,
    ) -> Result<()> {
      let mut scene =
        vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
      let result = self.scene(&mut scene, input, source);
      scene.finish()?;
      result
    }
  }
}
pub use preview_view_previewdraw::PreviewDraw;

pub mod reading_preview_readingdraw {
  use super::{
    Graphics, Markup, Overlays, Pages, Raster, Result, TextSource, Write,
  };
  use vpdf_ui::reading::View;
  pub trait ReadingDraw<S: TextSource + Pages + Markup + Raster> {
    fn draw_preview(
      &mut self,
      overlays: &mut Overlays,
      graphics: &mut Graphics,
      out: &mut impl Write,
    ) -> Result<()>;
  }
  impl<S: TextSource + Pages + Markup + Raster> ReadingDraw<S> for View<'_, S> {
    fn draw_preview(
      &mut self,
      overlays: &mut Overlays,
      graphics: &mut Graphics,
      out: &mut impl Write,
    ) -> Result<()> {
      let mut scene =
        vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
      let result = self.scene_preview(overlays, &mut scene);
      scene.finish()?;
      result
    }
  }
}
pub use reading_preview_readingdraw::ReadingDraw;

pub mod panel_view_paneldraw {
  use super::{At, Graphics, Panel, Result, Screen, Write};
  pub trait PanelDraw {
    fn draw(
      &mut self,
      out: &mut impl Write,
      at: At,
      width: u16,
      screen: Screen,
      graphics: Option<&mut Graphics>,
      now: u64,
    ) -> Result<()>;
    fn list(
      &mut self,
      out: &mut impl Write,
      at: At,
      width: u16,
      height: u16,
    ) -> Result<()>;
  }
  impl PanelDraw for Panel {
    fn draw(
      &mut self,
      out: &mut impl Write,
      at: At,
      width: u16,
      screen: Screen,
      graphics: Option<&mut Graphics>,
      now: u64,
    ) -> Result<()> {
      let pictures = graphics.is_some();
      let mut scene = vpdf_terminal::painter::Painter::new(out);
      if let Some(graphics) = graphics {
        scene = scene.with_graphics(graphics);
      }
      let result = self.scene(
        &mut scene,
        vpdf_ui::panel::recent::Frame { at, width, screen, now },
        pictures,
      );
      scene.finish()?;
      result
    }
    fn list(
      &mut self,
      out: &mut impl Write,
      at: At,
      width: u16,
      height: u16,
    ) -> Result<()> {
      let mut painter = vpdf_terminal::painter::Painter::new(out);
      self.scene_list(&mut painter, at, width, height);
      painter.finish()?;
      Ok(())
    }
  }
}
pub use panel_view_paneldraw::PanelDraw;

pub mod panel_rows_rowdraw {
  use super::{Result, Role, Write};
  use vpdf_ui::panel::Row;
  pub trait RowDraw {
    fn write(
      &self,
      out: &mut impl Write,
      base: Option<Role>,
      room: usize,
      keys: usize,
      query: &str,
    ) -> Result<usize>;
  }
  impl RowDraw for Row {
    fn write(
      &self,
      out: &mut impl Write,
      base: Option<Role>,
      room: usize,
      keys: usize,
      query: &str,
    ) -> Result<usize> {
      let mut painter = vpdf_terminal::painter::Painter::new(out);
      let width = self.scene(&mut painter, base, room, keys, query);
      painter.finish()?;
      Ok(width)
    }
  }
}
pub use panel_rows_rowdraw::RowDraw;

pub mod panel_recent_shelfdraw {
  use super::{Graphics, Panel, Result, Shelf, Write};
  use vpdf_ui::panel::recent::Frame;
  pub trait ShelfDraw {
    fn draw(
      &mut self,
      panel: &mut Panel,
      out: &mut impl Write,
      frame: Frame,
      graphics: Option<&mut Graphics>,
    ) -> Result<()>;
  }
  impl ShelfDraw for Shelf {
    fn draw(
      &mut self,
      panel: &mut Panel,
      out: &mut impl Write,
      frame: Frame,
      graphics: Option<&mut Graphics>,
    ) -> Result<()> {
      let pictures = graphics.is_some();
      let mut scene = vpdf_terminal::painter::Painter::new(out);
      if let Some(graphics) = graphics {
        scene = scene.with_graphics(graphics);
      }
      let result = self.scene(panel, &mut scene, frame, pictures);
      scene.finish()?;
      result
    }
  }
}
pub use panel_recent_shelfdraw::ShelfDraw;

pub mod grid {
  use super::{At, Result, Screen, Write};
  use vpdf_ui::grid::{Grid, Shown, scene, scene_cell};
  pub fn cell(
    out: &mut impl Write,
    label: &str,
    width: usize,
    keys: usize,
    shown: Shown,
  ) -> Result<()> {
    let mut scene = vpdf_terminal::painter::Painter::new(out);
    scene_cell(&mut scene, label, width, keys, shown);
    scene.finish()?;
    Ok(())
  }
  pub fn draw(
    out: &mut impl Write,
    (at, bottom): (At, u16),
    labels: &[String],
    grid: &Grid,
    (chosen, pointed): (Option<usize>, Option<usize>),
    keys: usize,
    screen: Screen,
  ) -> Result<()> {
    let mut scene = vpdf_terminal::painter::Painter::new(out);
    self::scene(
      &mut scene,
      (at, bottom),
      labels,
      grid,
      (chosen, pointed),
      keys,
      screen,
    );
    scene.finish()?;
    Ok(())
  }
}

#[path = "replay.rs"]
mod replay;
pub use replay::replay_frame;
