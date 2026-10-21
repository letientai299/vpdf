//! Shared benchmark step language.

mod catalog;
pub use catalog::{Scenario, Source};

use crate::Result;
use std::time::Duration;
use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Eq, PartialEq)]
pub enum Step {
  Keys(Vec<KeyEvent>),
  Wheel(i16, i16),
  Resize(u32, u32),
  Place(i32, i32),
  Mark(String),
  Wait(Wait),
  Repeat { duration: Duration, hz: u32, step: Box<Self> },
  Mix { seed: u64, keys: Vec<Vec<KeyEvent>> },
  Quit,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Wait {
  Complete,
  Idle,
  Time(Duration),
}

impl Step {
  pub fn parse(line: &str) -> Result<Option<Self>> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
      return Ok(None);
    }
    let (verb, rest) =
      line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    let step = match verb {
      "key" => Self::Keys(keys(rest)?),
      "wheel" => {
        let (x, y) = pair::<i16>(rest)?;
        anyhow::ensure!(
          x.unsigned_abs() <= 64 && y.unsigned_abs() <= 64,
          "Scenario wheel exceeds turn limit"
        );
        Self::Wheel(x, y)
      }
      "resize" => {
        let (width, height) = pair(rest)?;
        anyhow::ensure!(
          width > 0 && height > 0 && width <= 8192 && height <= 8192,
          "Invalid scenario size"
        );
        Self::Resize(width, height)
      }
      "place" => {
        let (x, y) = pair(rest)?;
        Self::Place(x, y)
      }
      "mark" if !rest.is_empty() => Self::Mark(rest.into()),
      "wait" => Self::Wait(match rest {
        "complete" => Wait::Complete,
        "idle" => Wait::Idle,
        _ => Wait::Time(duration(rest)?),
      }),
      "repeat" => repeat(rest)?,
      "mix" => {
        let (seed, rest) = rest
          .split_once(char::is_whitespace)
          .ok_or_else(|| anyhow::anyhow!("Missing scenario mix choices"))?;
        let keys = rest
          .split('|')
          .map(|choice| keys(choice.trim()))
          .collect::<Result<Vec<_>>>()?;
        Self::Mix { seed: seed.parse()?, keys }
      }
      "quit" if rest.is_empty() => Self::Quit,
      _ => anyhow::bail!("Unknown scenario step: {line}"),
    };
    Ok(Some(step))
  }
}

fn pair<T: std::str::FromStr>(text: &str) -> Result<(T, T)> {
  let words: Vec<_> = text.split_whitespace().collect();
  let [a, b] = words.as_slice() else {
    anyhow::bail!("Scenario step needs two coordinates");
  };
  let parse = |word: &str| {
    word
      .parse()
      .map_err(|_| anyhow::anyhow!("Invalid scenario coordinate: {word}"))
  };
  Ok((parse(a)?, parse(b)?))
}

fn duration(text: &str) -> Result<Duration> {
  let (number, multiplier) = if let Some(number) = text.strip_suffix("ms") {
    (number, 1)
  } else if let Some(number) = text.strip_suffix('s') {
    (number, 1000)
  } else {
    (text, 1)
  };
  let millis = number
    .parse::<u64>()?
    .checked_mul(multiplier)
    .ok_or_else(|| anyhow::anyhow!("Scenario duration overflow"))?;
  Ok(Duration::from_millis(millis))
}

fn repeat(text: &str) -> Result<Step> {
  let fields = text.split_once(char::is_whitespace).and_then(|(span, rest)| {
    let (rate, rest) = rest.trim_start().split_once(char::is_whitespace)?;
    Some((span, rate, rest.trim_start()))
  });
  let Some((span, rate, rest)) = fields else {
    anyhow::bail!("Scenario repeat needs duration, rate, step");
  };
  let duration = duration(span)?;
  let hz = rate
    .strip_prefix('@')
    .and_then(|rate| rate.parse::<u32>().ok())
    .filter(|rate| *rate > 0 && *rate <= 1000)
    .ok_or_else(|| anyhow::anyhow!("Invalid scenario repeat rate"))?;
  let step = Step::parse(rest)?
    .ok_or_else(|| anyhow::anyhow!("Missing scenario repeated step"))?;
  anyhow::ensure!(
    matches!(step, Step::Keys(_) | Step::Wheel(_, _) | Step::Mix { .. }),
    "Repeat needs keys, wheel, or mix"
  );
  anyhow::ensure!(!duration.is_zero(), "Scenario repeat duration is zero");
  Ok(Step::Repeat { duration, hz, step: Box::new(step) })
}

fn keys(text: &str) -> Result<Vec<KeyEvent>> {
  let keys = text.split_whitespace().map(key).collect::<Result<Vec<_>>>()?;
  anyhow::ensure!(!keys.is_empty(), "Scenario key list is empty");
  Ok(keys)
}

fn key(text: &str) -> Result<KeyEvent> {
  let mut name = text;
  let mut modifiers = KeyModifiers::NONE;
  while let Some((prefix, rest)) = name.split_once('-') {
    modifiers |= match prefix {
      "C" => KeyModifiers::CONTROL,
      "S" => KeyModifiers::SHIFT,
      "A" => KeyModifiers::ALT,
      "D" => KeyModifiers::SUPER,
      _ => break,
    };
    name = rest;
  }
  let code = match name.to_ascii_lowercase().as_str() {
    "space" => KeyCode::Char(' '),
    "enter" | "return" => KeyCode::Enter,
    "esc" | "escape" => KeyCode::Esc,
    "tab" if modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
    "tab" => KeyCode::Tab,
    "backspace" => KeyCode::Backspace,
    "delete" => KeyCode::Delete,
    "up" => KeyCode::Up,
    "down" => KeyCode::Down,
    "left" => KeyCode::Left,
    "right" => KeyCode::Right,
    "home" => KeyCode::Home,
    "end" => KeyCode::End,
    "pageup" => KeyCode::PageUp,
    "pagedown" => KeyCode::PageDown,
    _ => {
      let mut chars = name.chars();
      let first =
        chars.next().ok_or_else(|| anyhow::anyhow!("Missing scenario key"))?;
      anyhow::ensure!(chars.next().is_none(), "Unknown scenario key: {name}");
      KeyCode::Char(first)
    }
  };
  if code == KeyCode::Char('_')
    && modifiers.intersects(
      KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
    )
  {
    modifiers |= KeyModifiers::SHIFT;
  }
  Ok(KeyEvent::new(code, modifiers))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn repeat_accepts_whitespace() -> Result<()> {
    for repeated in ["key g o", "wheel 0 -1", "mix 7 C-d | S-space"] {
      let expected = Step::parse(&format!("repeat 2s @60 {repeated}"))?;
      for prefix in ["repeat 2s   @60   ", "repeat\t2s\t\t@60\t \t"] {
        assert_eq!(Step::parse(&format!("{prefix}{repeated}"))?, expected);
      }
    }
    for line in ["repeat 2s   ", "repeat 2s\t\t@60\t \t"] {
      assert!(Step::parse(line).is_err(), "{line}");
    }
    Ok(())
  }

  #[test]
  fn step_lines() -> Result<()> {
    let cases = [
      (
        "key C-d S-space g o",
        Step::Keys(vec![
          KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
          KeyEvent::new(KeyCode::Char(' '), KeyModifiers::SHIFT),
          KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
          KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
        ]),
      ),
      ("wheel 0 -1", Step::Wheel(0, -1)),
      ("resize 1100 800", Step::Resize(1100, 800)),
      ("place -100 20", Step::Place(-100, 20)),
      ("mark phase one", Step::Mark("phase one".into())),
      ("wait complete", Step::Wait(Wait::Complete)),
      ("wait idle", Step::Wait(Wait::Idle)),
      ("wait 25", Step::Wait(Wait::Time(Duration::from_millis(25)))),
      ("wait 2s", Step::Wait(Wait::Time(Duration::from_secs(2)))),
      ("quit", Step::Quit),
    ];
    for (text, want) in cases {
      assert_eq!(Step::parse(text)?, Some(want), "{text}");
    }
    assert!(matches!(
      Step::parse("repeat 2s @60 mix 7 C-d | S-space")?,
      Some(Step::Repeat { hz: 60, .. })
    ));
    for line in ["", " # metadata"] {
      assert_eq!(Step::parse(line)?, None);
    }
    for line in [
      "key",
      "key C-",
      "key B-x",
      "wheel 65 0",
      "wheel 0 -32768",
      "resize 0 1",
      "place 1 2 3",
      "mark",
      "wait -1",
      "quit now",
      "repeat 1s @0 key j",
      "repeat 1s @60 quit",
      "mix 7 j |",
      "wait 18446744073709551615s",
    ] {
      assert!(Step::parse(line).is_err(), "{line}");
    }
    Ok(())
  }
}
