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
    (Picker::Terminal { fzf: true }, ":fzf — Find a PDF", ":open"),
    (Picker::Native, ":open — Open a PDF", ":fzf"),
    (Picker::Terminal { fzf: false }, ":help install", ":fzf"),
  ] {
    let mut home =
      Home::new(vec![], &[], crate::testing::screen::cells(80, 24), picker);
    let scene = render(&mut home)?;
    let text = crate::testing::text(&scene);
    for hint in [
      "Vpdf",
      "A PDF reader with Vim keys.",
      expected,
      ":edit <file> — Open by path",
      ":help — Learn the reader",
      ":q — Quit",
    ] {
      assert!(text.contains(hint), "{hint}: {text}");
    }
    assert!(!text.contains(absent), "{text}");
    assert!(!text.contains('│'), "{text}");
    assert!(!text.contains("RECENT"), "{text}");
    assert!(text.contains(" Vpdf "), "{text}");
    if matches!(picker, Picker::Terminal { fzf: false }) {
      assert!(text.contains("fzf unavailable; optional helpers"), "{text}");
    }
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
