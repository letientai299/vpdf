use crate::Result;
use crate::app::App;
use crate::pdf_text::PdfText;
use std::io::Write;
use vpdf_ui::paint::Marks;
use vpdf_ui::text_view::{Frame, Pane};

impl App {
  pub fn text(&mut self) -> Pane<'_, PdfText<'_>> {
    let frame = Frame { screen: self.screen, at: self.at, left: self.left };
    self.text_view.bind(PdfText(&mut self.doc), frame)
  }

  pub fn text_canvas(&mut self, out: &mut impl Write) -> Result<()> {
    let frame = Frame { screen: self.screen, at: self.at, left: self.left };
    let hovered = self.hover_segments();
    let paint = Marks {
      reader: &self.reader,
      matches: if self.search.hidden() { &[] } else { self.search.hits() },
      flash: self.reader.landing().glyph(&self.reader),
      landing: self.reader.landing().line(),
      cursorline: self.cursorline,
      dim: self.panel_focused(),
      hovered: &hovered,
    };
    self.text_view.bind(PdfText(&mut self.doc), frame).draw(out, &paint)
  }
}
