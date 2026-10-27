use super::*;
use vpdf_model::input::{KeyCode, KeyModifiers};

fn key(code: KeyCode) -> Event {
  Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn home(picker: Picker) -> Home {
  let paths = ["/missing/a.pdf", "/missing/b.pdf"];
  Home::new(
    paths
      .into_iter()
      .map(|path| {
        Row::new(path.into(), String::new(), Destination::File(path.into()))
      })
      .collect(),
    &[],
    crate::testing::screen::cells(120, 24),
    picker,
  )
}

#[test]
fn recent_home_selection_and_confirmation() -> Result<()> {
  for picker in [Picker::Native, Picker::Terminal { fzf: true }] {
    let mut home = home(picker);
    let mut history = History::default();
    for code in [KeyCode::Char(' '), KeyCode::Char('j')] {
      assert_eq!(home.event(key(code), &mut history)?, Effect::Stay);
    }
    assert_eq!(
      home.event(key(KeyCode::Enter), &mut history)?,
      Effect::Open("/missing/b.pdf".into())
    );
    home.event(key(KeyCode::Char('x')), &mut history)?;
    assert!(home.confirming());
    assert_eq!(
      home.event(Event::Paste("q".into()), &mut history)?,
      Effect::Stay
    );
    assert_eq!(
      home.event(key(KeyCode::Char(':')), &mut history)?,
      Effect::Stay
    );
    assert!(!home.typing());
    assert_eq!(
      home.event(key(KeyCode::Enter), &mut history)?,
      Effect::Forget(vec!["/missing/a.pdf".into()])
    );
    assert_eq!(history.commands(), [] as [String; 0]);
    assert_eq!(history.searches(), [] as [String; 0]);
  }
  Ok(())
}

#[test]
fn cancellation_keeps_home_selection() -> Result<()> {
  let mut home = home(Picker::Native);
  let mut history = History::default();
  for code in [
    KeyCode::Char(' '),
    KeyCode::Char('x'),
    KeyCode::Esc,
    KeyCode::Char('j'),
    KeyCode::Char('x'),
  ] {
    home.event(key(code), &mut history)?;
  }
  assert_eq!(
    home.event(key(KeyCode::Enter), &mut history)?,
    Effect::Forget(vec!["/missing/a.pdf".into()])
  );
  Ok(())
}

/// The confirm bar's buttons answer clicks on the home view.
#[test]
fn home_confirm_buttons_answer_clicks() -> Result<()> {
  use vpdf_model::input::{Mouse, MouseButton, MouseEvent, MouseEventKind};
  // (column, effect)
  let cases =
    [(25, Effect::Stay), (3, Effect::Forget(vec!["/missing/a.pdf".into()]))];
  for (column, effect) in cases {
    let mut home = home(Picker::Native);
    let mut history = History::default();
    home.event(key(KeyCode::Char('x')), &mut history)?;
    let mut scene = crate::testing::empty();
    home.scene(&mut scene, 0, |_| {})?;
    let kind = MouseEventKind::Down(MouseButton::Left);
    let press = MouseEvent::new(kind, column, 22);
    let got = home.event(Event::Mouse(Mouse::from(press)), &mut history)?;
    assert_eq!(got, effect, "{column}");
    assert!(!home.confirming(), "{column}");
  }
  Ok(())
}

/// Home shows the build lines; it has no fold keys.
#[test]
fn home_version_shows_every_group() {
  let mut home = home(Picker::Native);
  home
    .show_info(crate::keys::PanelKind::Version, "vpdf 1\n# Build\nsource abc");
  let info = home.info.as_ref().unwrap();
  assert_eq!(info.branch_closed(1), Some(false));
}

#[test]
fn home_resize_uses_reader_chord() -> Result<()> {
  let path = std::path::Path::new("/books/a.pdf");
  let entry = Entry {
    path: path.into(),
    cursor: crate::text::Position::default(),
    rotation: 0,
    used: 0,
    marks: 0,
  };
  let mut home = Home::new(
    vec![Row::recent(path, entry.cursor.page, None)],
    &[entry],
    crate::testing::screen::cells(120, 36),
    Picker::Native,
  );
  let mut scene = crate::testing::empty();
  home.scene(&mut scene, 0, |_| {})?;
  let before = home.shelf_mut().unwrap().start().unwrap();
  let mut history = History::default();
  home.event(
    Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
    &mut history,
  )?;
  home.event(key(KeyCode::Char('l')), &mut history)?;
  home.scene(&mut crate::testing::empty(), 0, |_| {})?;
  assert_eq!(home.shelf_mut().unwrap().start(), Some(before + 1));
  Ok(())
}

#[test]
fn resize_mode_explains_input() -> Result<()> {
  use crate::bindings::{Map, Sequence, Target};
  use crate::split::Direction;
  use crate::window::Window;
  for columns in [1, 7, 24, 120] {
    let mut home = home(Picker::Native);
    home.resize(crate::testing::screen::cells(columns, 24), At::default());
    let mut bindings = Bindings::default();
    bindings.set(
      Context::Resize,
      Map::new(vec![
        (
          Sequence::parse("L")?,
          Target::Reader(Action::Window(Window::Extend(Direction::Right))),
        ),
        (Sequence::parse("<Esc>")?, Target::Reader(Action::Cancel)),
      ])?,
    );
    home.set_bindings(bindings);
    let mut history = History::default();
    home.event(key(KeyCode::Char(' ')), &mut history)?;
    home.event(
      Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
      &mut history,
    )?;
    let mut scene = crate::testing::empty();
    home.scene(&mut scene, 0, |_| {})?;
    let scene = scene.into_scene();
    crate::testing::lines(&scene, columns);
    if columns >= 24 {
      let text = crate::testing::text(&scene);
      assert!(text.contains("RESIZE") && text.contains("L right"));
      assert!(
        crate::testing::spans(&scene)
          .iter()
          .any(|(role, text)| *role == Role::Key && text == "L")
      );
      assert!(!text.contains("q quit"));
    }
    home.event(key(KeyCode::Esc), &mut history)?;
    assert!(!home.keys.resizing());
    assert!(home.panel.clear_checked());
  }
  Ok(())
}
