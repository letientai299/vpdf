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
