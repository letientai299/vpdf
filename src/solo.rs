use crate::Result;
use crate::app::App;
use crate::trace::{Drawable, Observation};
use std::io::Write;
use std::ops::{Deref, DerefMut};
use std::path::Path;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::layout::Screen;

/// A pane drawn alone with images of its own, as tests and benches drive
/// one; a session shares its images among panes instead.
#[doc(hidden)]
pub struct Solo {
  pub app: App,
  pub graphics: Graphics,
}

impl Solo {
  pub fn new(app: App) -> Self {
    Self { app, graphics: Graphics::default() }
  }

  pub fn open(
    path: &Path,
    password: &str,
    text_mode: bool,
    screen: Screen,
  ) -> Result<Self> {
    App::with_screen(path, password, text_mode, screen).map(Self::new)
  }

  /// One frame with this pane alone on screen.
  pub fn draw(&mut self, out: &mut impl Write) -> Result<()> {
    self.app.draw_alone(&mut self.graphics, out)
  }

  pub fn parts(&mut self) -> (&mut App, &mut Graphics) {
    (&mut self.app, &mut self.graphics)
  }

  pub fn raster_updates(&mut self) -> bool {
    self.app.raster_updates(&mut self.graphics)
  }
}

impl Deref for Solo {
  type Target = App;

  fn deref(&self) -> &App {
    &self.app
  }
}

impl DerefMut for Solo {
  fn deref_mut(&mut self) -> &mut App {
    &mut self.app
  }
}

impl Drawable for Solo {
  fn draw_frame(&mut self, out: &mut Vec<u8>) -> Result<()> {
    self.draw(out)
  }

  fn observe(&self) -> Observation {
    Observation::of(&self.app, &self.graphics)
  }
}
