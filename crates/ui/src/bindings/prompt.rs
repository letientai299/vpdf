use super::{Map, Sequence};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "action", deny_unknown_fields)]
pub enum PromptAction {
  #[serde(rename = "prompt.start")]
  Start,
  #[serde(rename = "prompt.end")]
  End,
  #[serde(rename = "prompt.move")]
  Move { forward: bool },
  #[serde(rename = "prompt.accept")]
  Accept,
  #[serde(rename = "prompt.word")]
  Word { forward: bool, shell: bool },
  #[serde(rename = "prompt.delete")]
  Delete { backward: bool },
  #[serde(rename = "prompt.delete-or-complete")]
  DeleteOrComplete,
  #[serde(rename = "prompt.kill")]
  Kill { to_start: bool },
  #[serde(rename = "prompt.delete-word")]
  DeleteWord { forward: bool, shell: bool },
  #[serde(rename = "prompt.yank")]
  Yank,
  #[serde(rename = "prompt.transpose")]
  Transpose,
  #[serde(rename = "prompt.history")]
  History { forward: bool, search: bool },
  #[serde(rename = "prompt.complete")]
  Complete { forward: bool },
  #[serde(rename = "prompt.undo")]
  Undo,
  #[serde(rename = "prompt.editor")]
  Editor,
  #[serde(rename = "prompt.submit")]
  Submit,
  #[serde(rename = "prompt.cancel")]
  Cancel,
}

impl PromptAction {
  pub const NAMES: &[&str] = &[
    "prompt.start",
    "prompt.end",
    "prompt.move",
    "prompt.accept",
    "prompt.word",
    "prompt.delete",
    "prompt.delete-or-complete",
    "prompt.kill",
    "prompt.delete-word",
    "prompt.yank",
    "prompt.transpose",
    "prompt.history",
    "prompt.complete",
    "prompt.undo",
    "prompt.editor",
    "prompt.submit",
    "prompt.cancel",
  ];

  pub(crate) fn defaults(password: bool) -> Map<Self> {
    let mut entries = Vec::new();
    for &(keys, action) in DEFAULTS {
      if password && keys.starts_with("<C-x>") {
        continue;
      }
      if password && keys == "<Tab>" {
        continue;
      }
      entries
        .push((Sequence::parse(keys).expect("invalid prompt default"), action));
    }
    aliases(&mut entries);
    Map::new(entries).expect("conflicting prompt defaults")
  }
}

fn aliases(entries: &mut Vec<(Sequence, PromptAction)>) {
  use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers as M};
  let originals = entries.clone();
  for (sequence, action) in originals {
    let mut variants = vec![sequence.keys().to_vec()];
    for (index, key) in sequence.keys().iter().enumerate() {
      if !key.modifiers.contains(M::CONTROL)
        || !matches!(key.code, KeyCode::Char(_))
      {
        continue;
      }
      for modifier in [M::SHIFT, M::ALT] {
        let more = variants.clone().into_iter().map(|mut keys| {
          keys[index].modifiers |= modifier;
          keys
        });
        variants.extend(more);
      }
    }
    for keys in variants {
      let sequence = Sequence::from_keys(&keys).expect("prompt alias");
      if !entries.iter().any(|(key, _)| *key == sequence) {
        entries.push((sequence, action));
      }
    }
  }
  for modifiers in super::sequence::modifiers() {
    for code in [
      KeyCode::Left,
      KeyCode::Right,
      KeyCode::Home,
      KeyCode::End,
      KeyCode::Up,
      KeyCode::Down,
      KeyCode::Backspace,
      KeyCode::Delete,
    ] {
      let Some(action) = named_action(code, modifiers) else { continue };
      let sequence = Sequence::from_keys(&[KeyEvent::new(code, modifiers)])
        .expect("prompt alias");
      if !entries.iter().any(|(key, _)| *key == sequence) {
        entries.push((sequence, action));
      }
    }
    for (code, action) in [
      (KeyCode::Esc, PromptAction::Cancel),
      (KeyCode::Enter, PromptAction::Submit),
    ] {
      let sequence = Sequence::from_keys(&[KeyEvent::new(code, modifiers)])
        .expect("prompt alias");
      if !entries.iter().any(|(key, _)| *key == sequence) {
        entries.push((sequence, action));
      }
    }
  }
}
fn named_action(
  code: vpdf_model::input::KeyCode,
  modifiers: vpdf_model::input::KeyModifiers,
) -> Option<PromptAction> {
  use vpdf_model::input::{KeyCode as K, KeyModifiers as M};
  let control = modifiers.contains(M::CONTROL);
  let alt = modifiers.contains(M::ALT);
  Some(match code {
    K::Left | K::Right if control || alt => {
      PromptAction::Word { forward: code == K::Right, shell: !control }
    }
    K::Left | K::Right => PromptAction::Move { forward: code == K::Right },
    K::Backspace if alt => {
      PromptAction::DeleteWord { forward: false, shell: true }
    }
    K::Backspace | K::Delete if !alt => {
      PromptAction::Delete { backward: code == K::Backspace }
    }
    K::Home if !alt => PromptAction::Start,
    K::End if !alt => PromptAction::End,
    K::Up | K::Down if !alt => {
      PromptAction::History { forward: code == K::Down, search: false }
    }
    _ => return None,
  })
}

const DEFAULTS: &[(&str, PromptAction)] = &[
  ("<C-a>", PromptAction::Start),
  ("<Home>", PromptAction::Start),
  ("<C-e>", PromptAction::End),
  ("<End>", PromptAction::End),
  ("<C-b>", PromptAction::Move { forward: false }),
  ("<Left>", PromptAction::Move { forward: false }),
  ("<C-f>", PromptAction::Move { forward: true }),
  ("<Right>", PromptAction::Move { forward: true }),
  ("<C-Space>", PromptAction::Accept),
  ("<C-h>", PromptAction::Delete { backward: true }),
  ("<BS>", PromptAction::Delete { backward: true }),
  ("<Del>", PromptAction::Delete { backward: false }),
  ("<C-d>", PromptAction::DeleteOrComplete),
  ("<C-k>", PromptAction::Kill { to_start: false }),
  ("<C-u>", PromptAction::Kill { to_start: true }),
  ("<C-w>", PromptAction::DeleteWord { forward: false, shell: false }),
  ("<C-y>", PromptAction::Yank),
  ("<C-t>", PromptAction::Transpose),
  ("<C-p>", PromptAction::History { forward: false, search: false }),
  ("<Up>", PromptAction::History { forward: false, search: false }),
  ("<C-n>", PromptAction::History { forward: true, search: false }),
  ("<Down>", PromptAction::History { forward: true, search: false }),
  ("<C-r>", PromptAction::History { forward: false, search: true }),
  ("<C-s>", PromptAction::History { forward: true, search: true }),
  ("<C-Left>", PromptAction::Word { forward: false, shell: false }),
  ("<C-Right>", PromptAction::Word { forward: true, shell: false }),
  ("<A-b>", PromptAction::Word { forward: false, shell: true }),
  ("<A-Left>", PromptAction::Word { forward: false, shell: true }),
  ("<A-f>", PromptAction::Word { forward: true, shell: true }),
  ("<A-Right>", PromptAction::Word { forward: true, shell: true }),
  ("<A-d>", PromptAction::DeleteWord { forward: true, shell: true }),
  ("<A-BS>", PromptAction::DeleteWord { forward: false, shell: true }),
  ("<Tab>", PromptAction::Complete { forward: true }),
  ("<S-Tab>", PromptAction::Complete { forward: false }),
  ("<C-_>", PromptAction::Undo),
  ("<C-7>", PromptAction::Undo),
  ("<C-/>", PromptAction::Undo),
  ("<C-x><C-u>", PromptAction::Undo),
  ("<C-x><C-e>", PromptAction::Editor),
  ("<C-x>e", PromptAction::Editor),
  ("<C-c>", PromptAction::Cancel),
  ("<C-g>", PromptAction::Cancel),
  ("<Esc>", PromptAction::Cancel),
  ("<C-[>", PromptAction::Cancel),
  ("<CR>", PromptAction::Submit),
  ("<C-m>", PromptAction::Submit),
  ("<C-j>", PromptAction::Submit),
];
