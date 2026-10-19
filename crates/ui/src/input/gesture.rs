use crate::Result;
use crate::source::Figures;
use crate::text::{Bounds, PageId, Position};
use vpdf_model::document::FigureSource;
use vpdf_reader::objects::Object;
use vpdf_reader::reader::{Reader, Selection, SelectionKind, TextSource};

/// What a click run's drag extends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Stretch {
  /// Words for `Character`, lines for `Line`.
  kind: SelectionKind,
  /// The clicked unit's ends.
  first: Position,
  last: Position,
}

/// An Alt-press region capture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sweep {
  page: PageId,
  /// Page point of the press.
  from: (f32, f32),
  /// Page point of the drag, once moved.
  to: Option<(f32, f32)>,
}

/// What the held left button does.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum State {
  /// Plain drags select glyphs.
  #[default]
  Glyphs,
  /// Click-run drags extend by unit.
  Stretch(Stretch),
  /// Alt-drags sweep a capture.
  Sweep(Sweep),
  /// Cancelled until the release.
  Dropped,
}

#[derive(Default)]
pub struct Gesture {
  state: State,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapshot(State);

pub enum Swept {
  Region(PageId, Bounds),
  Extend(Position),
}

impl Sweep {
  /// A rectangle with nonzero area.
  pub fn bounds(self) -> Option<Bounds> {
    let ((x0, y0), (x1, y1)) = (self.from, self.to?);
    let bounds =
      Bounds { x0: x0.min(x1), y0: y0.min(y1), x1: x0.max(x1), y1: y0.max(y1) };
    (bounds.width() >= 1.0 && bounds.height() >= 1.0).then_some(bounds)
  }

  pub fn page(self) -> PageId {
    self.page
  }
  pub fn corner(self) -> Option<(f32, f32)> {
    self.to
  }

  pub fn resolve(
    self,
    source: &mut (impl Figures + TextSource),
  ) -> Result<Swept> {
    if let Some(bounds) = self.bounds() {
      return Ok(Swept::Region(self.page, bounds));
    }
    let (x, y) = self.from;
    let figure = crate::figures::at(
      source,
      crate::figures::Point { page: self.page, x, y },
    )?;
    if let Some(figure) = figure.filter(|f| f.source == FigureSource::Graphic) {
      return Ok(Swept::Region(self.page, figure.bounds));
    }
    Ok(Swept::Extend(source.page_text(self.page)?.nearest(self.page, x, y)))
  }
}

impl Gesture {
  pub fn reset(&mut self) {
    self.state = State::Glyphs;
  }
  pub fn snapshot(&self) -> Snapshot {
    Snapshot(self.state)
  }
  pub fn sweep(&self) -> Option<Sweep> {
    match self.state {
      State::Sweep(sweep) => Some(sweep),
      _ => None,
    }
  }
  pub fn alt(&self) -> bool {
    matches!(self.state, State::Sweep(_) | State::Dropped)
  }
  pub fn dropped(&self) -> bool {
    self.state == State::Dropped
  }
  pub fn begin_sweep(&mut self, page: PageId, from: (f32, f32)) {
    self.state = State::Sweep(Sweep { page, from, to: None });
  }
  pub fn sweep_to(&mut self, (x, y): (f32, f32), shown: Bounds) {
    let to = (x.clamp(shown.x0, shown.x1), y.clamp(shown.y0, shown.y1));
    if let State::Sweep(sweep) = &mut self.state {
      sweep.to = Some(to);
    }
  }
  pub fn finish_sweep(&mut self) -> Option<Sweep> {
    match std::mem::take(&mut self.state) {
      State::Sweep(sweep) => Some(sweep),
      _ => None,
    }
  }
  pub fn drop_sweep(&mut self) -> bool {
    let held = self.sweep().is_some();
    if held {
      self.state = State::Dropped;
    }
    held
  }
  pub fn hint(&self) -> Option<&'static str> {
    self.sweep()?.bounds().map(|_| "Release captures; Esc cancels")
  }
  /// Click runs: target, word, line, paragraph.
  pub fn click(
    &mut self,
    reader: &mut Reader,
    pos: Position,
    clicks: u8,
    source: &mut impl TextSource,
  ) -> Result<bool> {
    reader.end_selection();
    reader.cursor = pos;
    reader.desired = None;
    self.state = State::Glyphs;
    let (kind, object) = match clicks {
      // U30: marks where `i`, `K`, `*` act.
      0 | 1 if reader.viewport_mode() => {
        return Ok(true);
      }
      0 | 1 => return Ok(false),
      2 => (SelectionKind::Character, Some(Object::Word)),
      3 => (SelectionKind::Line, None),
      _ => (SelectionKind::Line, Some(Object::Paragraph)),
    };
    let (first, last) = match object {
      Some(object) => Reader::object_span(pos, object, false, 1, source)?
        .unwrap_or((pos, pos)),
      None => (pos, pos),
    };
    reader.selection = Some(Selection { anchor: first, kind });
    reader.cursor = last;
    self.state = State::Stretch(Stretch { kind, first, last });
    Ok(false)
  }

  /// Extends by the clicked unit.
  pub fn drag(
    &mut self,
    reader: &mut Reader,
    pos: Position,
    source: &mut impl TextSource,
  ) -> Result<()> {
    reader.desired = None;
    let State::Stretch(Stretch { kind, first, last }) = self.state else {
      if reader.selection.is_none() {
        reader.visual(SelectionKind::Character);
      }
      reader.cursor = pos;
      return Ok(());
    };
    let (start, end) = match kind {
      SelectionKind::Character => {
        Reader::object_span(pos, Object::Word, false, 1, source)?
          .unwrap_or((pos, pos))
      }
      SelectionKind::Line => (pos, pos),
    };
    // The clicked unit stays whole.
    let (anchor, cursor) =
      if start < first { (last, start) } else { (first, end.max(last)) };
    reader.selection = Some(Selection { anchor, kind });
    reader.cursor = cursor;
    Ok(())
  }
}
