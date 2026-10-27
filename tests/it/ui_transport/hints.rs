#[path = "../../support/hints.rs"]
mod fixture;
use anyhow::Result;
use vpdf_model::text::Bounds;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::{paint::Frame, theme};
#[test]
fn recorded_labels_keep_backdrops() -> Result<()> {
  use vpdf_model::geometry::At;
  use vpdf_model::scene::Scene;
  let (mut hints, screen) = fixture::labels()?;
  for picked in [false, true] {
    if picked {
      hints.select(0);
    }
    for text in [false, true] {
      let frame = Frame { at: At::default(), screen, left: 0, text };
      let mut scene = Scene::new();
      hints.scene(&mut scene, frame);
      let mut actual = Vec::new();
      let mut graphics = Graphics::default();
      let mut painter = vpdf_terminal::painter::Painter::new(&mut actual)
        .with_graphics(&mut graphics);
      painter.paint(&scene);
      painter.finish()?;
      let mut expected = if picked {
        b"\x1b[3;4H\x1b[0;1;97;44m\x1b[0;97;44ma\x1b[0m".to_vec()
      } else {
        b"\x1b[3;4H\x1b[0;1;30;43m\x1b[0;30;43ma\x1b[0m".to_vec()
      };
      let fill = if picked { theme::BADGE_FILL } else { theme::HINT_FILL };
      if !text {
        Graphics::default().backdrop(
          Bounds { x0: 24.0, y0: 32.0, x1: 32.0, y1: 48.0 },
          fill.resolve(vpdf_model::theme::Colors::Palette(
            vpdf_model::theme::Palette::dark(),
          )),
          screen,
          0,
          &mut expected,
        )?;
      }
      assert_eq!(actual, expected);
    }
  }
  Ok(())
}
