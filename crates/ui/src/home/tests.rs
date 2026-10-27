use super::*;
use vpdf_model::input::{KeyCode, KeyModifiers};

fn render(home: &mut Home) -> Result<Vec<Op<'static>>> {
  let mut scene = crate::testing::empty();
  home.scene(&mut scene, 0, |_| panic!("Empty home requested cards"))?;
  Ok(scene.into_scene())
}

#[test]
fn welcome_matches_picker() -> Result<()> {
  for (picker, expected, absent) in [
    (Picker::Terminal { fzf: true }, ":fzf", ":open"),
    (Picker::Native, ":open", ":fzf"),
    (Picker::Terminal { fzf: false }, ":open", ":fzf"),
  ] {
    let mut home =
      Home::new(vec![], &[], crate::testing::screen::cells(80, 24), picker);
    let scene = render(&mut home)?;
    let text = crate::testing::text(&scene);
    for hint in ["Vpdf", expected, ":edit <file>", "Learn the reader", "Quit"] {
      assert!(text.contains(hint), "{hint}: {text}");
    }
    assert!(!text.contains(absent), "{text}");
    assert!(!text.contains('│'), "{text}");
    assert!(!text.contains("RECENT"), "{text}");
    assert!(text.contains(" Vpdf "), "{text}");
    assert!(!text.contains("A PDF reader with Vim keys."));
  }
  Ok(())
}

#[test]
fn welcome_stays_inside_body() -> Result<()> {
  for (columns, rows) in [(80, 24), (40, 8), (24, 7), (16, 6), (7, 3), (1, 1)] {
    let screen = crate::testing::screen::cells(columns, rows);
    let at = At { column: 3, row: 4 };
    for picker in [Picker::Native, Picker::Terminal { fzf: false }] {
      let mut home = Home::new(vec![], &[], screen, picker);
      home.resize(screen, at);
      home.fail("Config warning".into());
      let scene = render(&mut home)?;
      let mut column = 0;
      for op in &scene {
        match op {
          Op::Move { row: r, column: c } => {
            column = *c;
            assert!(*r >= usize::from(at.row));
            assert!(*r < usize::from(at.row + rows));
            assert!(column >= usize::from(at.column));
          }
          Op::Text(text) => column += text.as_str().width(),
          Op::Spaces(count) => column += count,
          _ => {}
        }
        assert!(
          column <= usize::from(at.column + columns),
          "{columns}x{rows}: {op:?}"
        );
      }
      let spans = crate::testing::spans(&scene);
      assert!(spans.iter().any(|(role, _)| *role == Role::Error));
      if columns >= 24 {
        let text = crate::testing::text(&scene);
        assert!(text.contains("Config warning"), "{text}");
        for command in [":edit <file>", ":help", ":q"] {
          assert!(text.contains(command), "{columns}x{rows}: {text}");
        }
      }
    }
  }
  Ok(())
}

#[test]
fn empty_home_keeps_input() -> Result<()> {
  let mut home = Home::new(
    vec![],
    &[],
    crate::testing::screen::cells(80, 24),
    Picker::Native,
  );
  let mut history = History::default();
  let key = |code| Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
  assert_eq!(home.event(key(KeyCode::Enter), &mut history)?, Effect::Stay);
  assert_eq!(
    home.event(Event::Paste("paper.pdf".into()), &mut history)?,
    Effect::Paste("paper.pdf".into())
  );
  home.event(key(KeyCode::Char(':')), &mut history)?;
  home.event(Event::Paste("op".into()), &mut history)?;
  assert_eq!(
    home.event(key(KeyCode::Tab), &mut history)?,
    Effect::Complete("op".into())
  );
  home.complete_prompt(0, vec!["open".into()]);
  assert!(crate::testing::text(&render(&mut home)?).contains(":open"));
  assert_eq!(
    home.event(key(KeyCode::Enter), &mut history)?,
    Effect::Command("open".into())
  );
  assert_eq!(
    home.event(key(KeyCode::F(1)), &mut history)?,
    Effect::Command("help".into())
  );
  Ok(())
}

#[test]
fn zero_size_skips_drawing() -> Result<()> {
  use vpdf_model::input::{Button, Mouse, MouseKind, Spot};
  for (columns, rows) in [(0, 0), (0, 8), (20, 0)] {
    let mut home = Home::new(
      vec![],
      &[],
      crate::testing::screen::cells(columns, rows),
      Picker::Native,
    );
    assert_eq!(render(&mut home)?, [] as [Op<'_>; 0]);
    assert_eq!(
      home.event(
        Event::Mouse(Mouse {
          kind: MouseKind::Press(Button::LEFT),
          at: Spot::Cell { column: 0, row: 0 },
          modifiers: KeyModifiers::NONE,
        }),
        &mut History::default(),
      )?,
      Effect::Stay
    );
  }
  Ok(())
}

#[test]
fn welcome_columns_and_fallback() -> Result<()> {
  let mut home = Home::new(
    vec![],
    &[],
    crate::testing::screen::cells(80, 28),
    Picker::Native,
  );
  let scene = render(&mut home)?;
  let lines = crate::testing::lines(&scene, 80);
  let descriptions: Vec<_> = lines
    .values()
    .filter_map(|line| {
      ["Open a PDF", "Open by path", "Learn the reader", "Quit"]
        .iter()
        .find_map(|description| line.find(description))
    })
    .collect();
  assert_eq!(descriptions.len(), 4);
  assert!(descriptions.iter().all(|column| *column == descriptions[0]));
  home
    .shelf_mut()
    .unwrap()
    .set_logo(vpdf_model::image::Rgb::rgba(1, 1, vec![0, 0, 0, 0]).unwrap());
  assert!(render(&mut home)?.iter().any(|op| matches!(op, Op::Picture(_))));
  home.set_pictures(false);
  let scene = render(&mut home)?;
  assert!(!scene.iter().any(|op| matches!(op, Op::Picture(_))));
  assert!(crate::testing::text(&scene).contains("Vpdf"));
  Ok(())
}

#[test]
fn footer_uses_bound_keys_and_clicks() -> Result<()> {
  use crate::bindings::{Map, Sequence, Target};
  use vpdf_model::input::{Mouse, MouseButton, MouseEvent, MouseEventKind};
  let mut home = Home::new(
    vec![],
    &[],
    crate::testing::screen::cells(24, 8),
    Picker::Terminal { fzf: false },
  );
  let mut bindings = Bindings::default();
  bindings.set(
    Context::Home,
    Map::new(vec![(Sequence::parse("H")?, Target::Reader(Action::Help))])?,
  );
  home.set_bindings(bindings);
  let footer = home.footer();
  assert_eq!(footer, ":open  H help");
  let mut history = History::default();
  assert_eq!(
    home.event(
      Event::Key(KeyEvent::new(KeyCode::Char('H'), KeyModifiers::NONE)),
      &mut history
    )?,
    Effect::Command("help".into())
  );
  let click =
    Mouse::from(MouseEvent::new(MouseEventKind::Down(MouseButton::Left), 1, 7));
  assert_eq!(
    home.event(Event::Mouse(click), &mut history)?,
    Effect::Command("open".into())
  );
  Ok(())
}
