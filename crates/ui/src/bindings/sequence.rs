use std::fmt;
use vpdf_model::input::{
  KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Sequence(Vec<KeyEvent>);

impl Sequence {
  pub fn parse(text: &str) -> crate::Result<Self> {
    let mut keys = Vec::new();
    let mut rest = text;
    while let Some(ch) = rest.chars().next() {
      if ch == '<' {
        let end = rest
          .find('>')
          .ok_or_else(|| anyhow::anyhow!("Unclosed key notation"))?;
        keys.push(chord(&rest[1..end])?);
        rest = &rest[end + 1..];
      } else {
        anyhow::ensure!(!ch.is_control(), "Control character in binding");
        keys.push(canonical(KeyEvent::new(
          KeyCode::Char(ch),
          KeyModifiers::NONE,
        )));
        rest = &rest[ch.len_utf8()..];
      }
    }
    anyhow::ensure!(!keys.is_empty(), "Empty binding");
    Ok(Self(keys))
  }

  pub fn from_keys(keys: &[KeyEvent]) -> crate::Result<Self> {
    anyhow::ensure!(!keys.is_empty(), "Empty binding");
    anyhow::ensure!(
      keys.iter().all(|key| match key.code {
        KeyCode::Char(ch) => !ch.is_control(),
        KeyCode::F(number) => (1..=35).contains(&number),
        _ => true,
      }),
      "Unsupported key event"
    );
    let sequence = Self(keys.iter().copied().map(canonical).collect());
    fmt::write(&mut String::new(), format_args!("{sequence}"))
      .map_err(|_| anyhow::anyhow!("Unsupported key event"))?;
    Ok(sequence)
  }

  pub fn keys(&self) -> &[KeyEvent] {
    &self.0
  }

  pub fn starts_with(&self, prefix: &[KeyEvent]) -> bool {
    self.0.starts_with(prefix)
  }
}

impl fmt::Display for Sequence {
  fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
    for key in &self.0 {
      let modifiers = key.modifiers;
      let name = match key.code {
        KeyCode::Char(' ') => "Space".into(),
        KeyCode::Char('<') => "lt".into(),
        KeyCode::Char(ch) => ch.to_string(),
        KeyCode::Esc => "Esc".into(),
        KeyCode::Enter => "CR".into(),
        KeyCode::Tab => "Tab".into(),
        KeyCode::Backspace => "BS".into(),
        KeyCode::Delete => "Del".into(),
        KeyCode::Insert => "Insert".into(),
        KeyCode::Left => "Left".into(),
        KeyCode::Right => "Right".into(),
        KeyCode::Up => "Up".into(),
        KeyCode::Down => "Down".into(),
        KeyCode::Home => "Home".into(),
        KeyCode::End => "End".into(),
        KeyCode::PageUp => "PageUp".into(),
        KeyCode::PageDown => "PageDown".into(),
        KeyCode::F(number) => format!("F{number}"),
        _ => return Err(fmt::Error),
      };
      if matches!(key.code, KeyCode::Char(ch) if ch != ' ' && ch != '<')
        && modifiers.is_empty()
      {
        out.write_str(&name)?;
        continue;
      }
      out.write_str("<")?;
      for (flag, prefix) in [
        (KeyModifiers::CONTROL, "C-"),
        (KeyModifiers::ALT, "A-"),
        (KeyModifiers::SHIFT, "S-"),
        (KeyModifiers::SUPER, "D-"),
        (KeyModifiers::HYPER, "H-"),
        (KeyModifiers::META, "Meta-"),
      ] {
        if modifiers.contains(flag) {
          out.write_str(prefix)?;
        }
      }
      write!(out, "{name}>")?;
    }
    Ok(())
  }
}

pub(super) fn canonical(mut key: KeyEvent) -> KeyEvent {
  key.kind = KeyEventKind::Press;
  key.state = KeyEventState::NONE;
  if key.code == KeyCode::BackTab {
    key.code = KeyCode::Tab;
    key.modifiers |= KeyModifiers::SHIFT;
  }
  if let KeyCode::Char(mut ch) = key.code {
    if key.modifiers.contains(KeyModifiers::SHIFT) && ch.is_ascii_alphabetic() {
      ch.make_ascii_uppercase();
    }
    if ch.is_ascii_uppercase() {
      key.modifiers = key.modifiers - KeyModifiers::SHIFT;
    }
    key.code = KeyCode::Char(ch);
  }
  key
}

pub(super) fn modifiers() -> Vec<KeyModifiers> {
  let mut modifiers = vec![KeyModifiers::NONE];
  for flag in [
    KeyModifiers::CONTROL,
    KeyModifiers::SHIFT,
    KeyModifiers::ALT,
    KeyModifiers::SUPER,
    KeyModifiers::HYPER,
    KeyModifiers::META,
  ] {
    let added: Vec<_> = modifiers.iter().map(|value| *value | flag).collect();
    modifiers.extend(added);
  }
  modifiers
}

fn chord(text: &str) -> crate::Result<KeyEvent> {
  let mut rest = text;
  let mut modifiers = KeyModifiers::NONE;
  while let Some((prefix, tail)) = rest.split_once('-') {
    let flag = match prefix.to_ascii_lowercase().as_str() {
      "c" | "ctrl" => KeyModifiers::CONTROL,
      "a" | "m" => KeyModifiers::ALT,
      "s" => KeyModifiers::SHIFT,
      "d" | "super" => KeyModifiers::SUPER,
      "h" => KeyModifiers::HYPER,
      "meta" => KeyModifiers::META,
      _ => break,
    };
    anyhow::ensure!(!modifiers.contains(flag), "Duplicate key modifier");
    modifiers |= flag;
    rest = tail;
  }
  let code = match rest.to_ascii_lowercase().as_str() {
    "space" => KeyCode::Char(' '),
    "lt" => KeyCode::Char('<'),
    "esc" => KeyCode::Esc,
    "cr" | "enter" => KeyCode::Enter,
    "tab" => KeyCode::Tab,
    "bs" | "backspace" => KeyCode::Backspace,
    "del" | "delete" => KeyCode::Delete,
    "insert" => KeyCode::Insert,
    "left" => KeyCode::Left,
    "right" => KeyCode::Right,
    "up" => KeyCode::Up,
    "down" => KeyCode::Down,
    "home" => KeyCode::Home,
    "end" => KeyCode::End,
    "pageup" => KeyCode::PageUp,
    "pagedown" => KeyCode::PageDown,
    _ if rest.starts_with(['F', 'f']) && rest.len() > 1 => {
      let number =
        rest[1..].parse::<u8>().ok().filter(|n| (1..=35).contains(n));
      KeyCode::F(number.ok_or_else(|| anyhow::anyhow!("Invalid function key"))?)
    }
    _ => {
      let mut chars = rest.chars();
      let ch = chars.next().filter(|ch| !ch.is_control());
      anyhow::ensure!(
        ch.is_some() && chars.next().is_none(),
        "Unknown key notation"
      );
      KeyCode::Char(ch.expect("validated character"))
    }
  };
  Ok(canonical(KeyEvent::new(code, modifiers)))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn notation_roundtrips() {
    for text in [
      "gg",
      "<Space>r",
      "<C-n>",
      "<C-A-Left>",
      "<D-S-a>",
      "<lt>",
      "<F12>",
      "<F>",
      "<C-S-f>",
      "<C-F>",
      "<D-F>",
      "é界",
      "<A-->",
      "<S-Space>",
      "<Meta-x>",
    ] {
      let sequence = Sequence::parse(text).unwrap();
      assert_eq!(
        Sequence::parse(&sequence.to_string()).unwrap(),
        sequence,
        "{text}"
      );
    }
    assert_eq!(
      Sequence::parse("<S-a>").unwrap(),
      Sequence::parse("A").unwrap()
    );
    assert_ne!(
      Sequence::parse("<C-i>").unwrap(),
      Sequence::parse("<Tab>").unwrap()
    );
    assert_ne!(
      Sequence::parse("<Left>").unwrap(),
      Sequence::parse("h").unwrap()
    );
  }

  #[test]
  fn notation_names_ignore_case() {
    for (lower, canonical) in [
      ("<c-w>", "<C-w>"),
      ("<CTRL-w>", "<C-w>"),
      ("<a-s-left>", "<A-S-Left>"),
      ("<esc>", "<Esc>"),
      ("<f1>", "<F1>"),
    ] {
      assert_eq!(
        Sequence::parse(lower).unwrap(),
        Sequence::parse(canonical).unwrap()
      );
    }
    assert_ne!(
      Sequence::parse("<c-w>").unwrap(),
      Sequence::parse("<c-W>").unwrap()
    );
    assert!(Sequence::parse("<c-C-w>").is_err());
  }

  #[test]
  fn malformed_notation_fails() {
    for text in [
      "",
      "<",
      "<C-",
      "<F0>",
      "<F36>",
      "<Unknown>",
      "<C-C-a>",
      "<>",
      "\n",
      "<C-abc>",
    ] {
      assert!(Sequence::parse(text).is_err(), "{text}");
    }
  }

  #[test]
  fn input_identity_ignores_protocol_state() {
    let event = KeyEvent {
      code: KeyCode::Char('j'),
      modifiers: KeyModifiers::NONE,
      kind: KeyEventKind::Repeat,
      state: KeyEventState::CAPS_LOCK,
    };
    assert_eq!(canonical(event), Sequence::parse("j").unwrap().keys()[0]);
    let backtab = KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE);
    assert_eq!(
      canonical(backtab),
      Sequence::parse("<S-Tab>").unwrap().keys()[0]
    );
  }
}
