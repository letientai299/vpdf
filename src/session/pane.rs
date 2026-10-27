use super::{App, At, Home, Result, Screen, Tabline};
use std::path::PathBuf;
use vpdf_model::text::Position;
use vpdf_ui::prompt::Prompt;

pub enum Body {
  Document(Box<App>),
  Recent(Box<Home>),
}

pub struct Alternate {
  pub path: PathBuf,
  pub position: Position,
  pub password: String,
  pub manual: Option<std::sync::Arc<vpdf_pdf::manual::Document>>,
}

pub struct Pane {
  pub id: u32,
  pub screen: Screen,
  pub at: At,
  pub body: Body,
  pub alternate: Option<Alternate>,
}

impl From<App> for Pane {
  fn from(app: App) -> Self {
    Self {
      id: app.id,
      screen: app.screen,
      at: app.at,
      body: Body::Document(Box::new(app)),
      alternate: None,
    }
  }
}

impl Pane {
  pub fn document(&self) -> Option<&App> {
    match &self.body {
      Body::Document(app) => Some(app),
      Body::Recent(_) => None,
    }
  }

  pub fn document_mut(&mut self) -> Option<&mut App> {
    match &mut self.body {
      Body::Document(app) => Some(app),
      Body::Recent(_) => None,
    }
  }

  pub fn recent(&self) -> Option<&Home> {
    match &self.body {
      Body::Recent(home) => Some(home),
      Body::Document(_) => None,
    }
  }

  pub fn recent_mut(&mut self) -> Option<&mut Home> {
    match &mut self.body {
      Body::Recent(home) => Some(home),
      Body::Document(_) => None,
    }
  }

  pub(super) fn is_help(&self) -> bool {
    self.document().is_some_and(|app| app.manual.is_some())
  }

  pub fn is_recent(&self) -> bool {
    matches!(self.body, Body::Recent(_))
  }

  pub fn name(&self) -> String {
    self.document().map_or_else(|| "Recent".into(), App::name)
  }

  pub fn title(&self) -> String {
    self.document().map_or_else(|| "Vpdf — Recent".into(), App::title)
  }

  pub(super) fn fail(&mut self, message: String) {
    match &mut self.body {
      Body::Document(app) => app.error = Some(message),
      Body::Recent(home) => home.view.fail(message),
    }
  }

  pub(super) fn prompt(&self) -> Option<&Prompt> {
    match &self.body {
      Body::Document(app) => app.overlays.prompt(),
      Body::Recent(home) => home.view.prompt(),
    }
  }

  pub(super) fn prompt_mut(&mut self) -> Option<&mut Prompt> {
    match &mut self.body {
      Body::Document(app) => app.overlays.prompt_mut(),
      Body::Recent(home) => home.view.prompt_mut(),
    }
  }

  pub(super) fn open_prompt(&mut self, prompt: Prompt) {
    match &mut self.body {
      Body::Document(app) => app.overlays.open_prompt(prompt),
      Body::Recent(home) => home.view.open_prompt(prompt),
    }
  }

  pub(super) fn blur(&mut self) -> Result<()> {
    match &mut self.body {
      Body::Document(app) => app.blur(),
      Body::Recent(home) => {
        home.view.reset_input();
        Ok(())
      }
    }
  }

  pub(super) fn layout(
    &mut self,
    screen: Screen,
    at: At,
    focused: bool,
    tabline: Tabline,
  ) -> Result<()> {
    self.screen = screen;
    self.at = at;
    match &mut self.body {
      Body::Document(app) => {
        app.blank = false;
        app.focused = focused;
        app.tabline = tabline;
        let before = (app.screen.columns, app.screen.rows, app.screen.origin);
        if before != (screen.columns, screen.rows, screen.origin)
          || app.at != at
        {
          app.overlays.resized();
          app.screen = screen;
          app.at = at;
          app.tiles.invalidate();
          app.relayout()?;
        }
      }
      Body::Recent(home) => {
        home.view.resize(screen, at);
        home.set_tabline(tabline, focused);
      }
    }
    Ok(())
  }

  pub(super) fn chrome_at(
    &self,
    column: u16,
    row: u16,
  ) -> Option<vpdf_ui::chrome::Item> {
    match &self.body {
      Body::Document(app) => app.chrome_at(column, row),
      Body::Recent(home) => home.view.chrome_at(column, row),
    }
  }

  pub fn pointer_pane(&self) -> vpdf_ui::pointer::Pane {
    self.document().map_or_else(
      || vpdf_ui::pointer::Pane {
        id: self.id,
        rect: vpdf_ui::split::Rect {
          column: self.at.column,
          row: self.at.row,
          columns: self.screen.columns,
          rows: self.screen.rows,
        },
        panel: self.screen.columns,
        overlay: 0,
        overview: false,
        preview: None,
        menu: None,
        actions: None,
      },
      App::pointer_pane,
    )
  }

  pub(super) fn scene_pane<'a, S: vpdf_model::scene::Surface<'a>>(
    &mut self,
    scene: &mut S,
    snap: bool,
  ) -> Result<()>
  where
    S::Error: Into<anyhow::Error>,
  {
    match &mut self.body {
      Body::Document(app) => app.scene_pane(scene, snap),
      Body::Recent(home) => home.scene(scene).map_err(Into::into),
    }
  }

  pub(super) fn paint_pane(
    &mut self,
    painter: &mut vpdf_terminal::painter::Painter<'_, impl std::io::Write>,
    snap: bool,
  ) -> Result<()> {
    match &mut self.body {
      Body::Document(app) => app.paint_pane(painter, snap),
      Body::Recent(home) => home.scene(painter),
    }
  }

  pub fn draw_alone(
    &mut self,
    graphics: &mut vpdf_terminal::graphics::Graphics,
    out: &mut impl std::io::Write,
  ) -> Result<()> {
    match &mut self.body {
      Body::Document(app) => app.draw_alone(graphics, out),
      Body::Recent(home) => {
        write!(out, "{}", vpdf_terminal::terminal::frame_start())?;
        graphics.begin(out)?;
        crate::presentation::draw(out, Some(graphics), |scene| {
          home.scene(scene)
        })??;
        graphics.end(out)?;
        write!(out, "{}", vpdf_terminal::terminal::frame_end())?;
        out.flush()?;
        Ok(())
      }
    }
  }
}
