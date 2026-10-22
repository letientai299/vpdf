use super::{Action, BINDINGS, PanelKind, key_for, notation, parse, token};
use crate::panel::Panel;

pub(crate) fn text() -> String {
  fill(include_str!("../help.txt"))
}

/// Replaces `{description}` with its keys; `{{` and `}}` escape.
fn fill(template: &str) -> String {
  let mut text = String::with_capacity(template.len());
  let mut rest = template;
  while let Some(start) = rest.find(['{', '}']) {
    let (head, tail) = rest.split_at(start);
    text.push_str(head);
    let (brace, tail) = tail.split_at(1);
    if let Some(after) = tail.strip_prefix(brace) {
      text.push_str(brace);
      rest = after;
      continue;
    }
    let placeholder = tail.split_once('}').filter(|_| brace == "{");
    let (name, after) = placeholder.expect("Unmatched help brace");
    text.push_str(&keys(name));
    rest = after;
  }
  text.push_str(rest);
  text
}

/// Keys for a placeholder: `|` groups, `all ` lists aliases.
fn keys(name: &str) -> String {
  match name {
    "windows" => fill(include_str!("reference/windows.txt")),
    "panel follow" => panel(PanelKind::Outline, Action::Follow),
    "panel note" => panel(PanelKind::Annotations, Action::Peek),
    "panel keep" => panel(PanelKind::Outline, Action::Swap),
    "panel yank" => panel(PanelKind::Outline, Action::YankLine),
    "panel close" => panel(PanelKind::Outline, Action::ClosePanel),
    "recent follow" => panel(PanelKind::Recent, Action::Follow),
    _ => match name.strip_prefix("all ") {
      Some(description) => aliases(described(description)),
      None => group(&name.split('|').map(described).collect::<Vec<_>>()),
    },
  }
}

/// The action bound with `description`.
fn described(description: &str) -> Action {
  let binding = BINDINGS.iter().find(|(.., text)| *text == description);
  binding.map(|(_, action, _)| *action).expect("Missing help binding")
}

fn aliases(action: Action) -> String {
  BINDINGS
    .iter()
    .filter(|(_, bound, _)| *bound == action)
    .map(|(keys, ..)| notation(keys))
    .collect::<Vec<_>>()
    .join(" ")
}

fn panel(kind: PanelKind, action: Action) -> String {
  let panel = if kind == PanelKind::Recent {
    Panel::recent(Vec::new(), &[])
  } else {
    Panel::new(kind, Vec::new())
  };
  notation(panel.shortcut(action).expect("Missing panel help binding"))
}

fn group(actions: &[Action]) -> String {
  let names: Vec<_> = actions
    .iter()
    .map(|&action| key_for(action).expect("Missing help binding"))
    .collect();
  let events: Vec<_> = names
    .iter()
    .map(|name| parse(name).expect("Invalid help binding"))
    .collect();
  let Some(first) = events.first() else { return String::new() };
  let prefix: String = first
    .iter()
    .enumerate()
    .take(first.len().saturating_sub(1))
    .take_while(|(index, key)| {
      events.iter().all(|keys| keys.get(*index) == Some(*key))
    })
    .filter_map(|(_, key)| token(*key))
    .collect();
  names
    .iter()
    .enumerate()
    .map(|(index, name)| {
      notation(if index == 0 {
        name
      } else {
        name.strip_prefix(&prefix).unwrap_or(name)
      })
    })
    .collect::<Vec<_>>()
    .join("/")
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::split::Axis;
  use crate::window::Window as W;

  #[test]
  fn groups_elide_only_whole_shared_keys() {
    assert_eq!(
      group(&[
        Action::Window(W::Split(Axis::Rows)),
        Action::Window(W::Split(Axis::Columns))
      ]),
      "<C-w>s/v"
    );
    assert_eq!(
      group(&[Action::History(false), Action::History(true)]),
      "<C-o>/<C-i>"
    );
    assert_eq!(
      group(&[Action::Window(W::Split(Axis::Rows)), Action::Quit]),
      "<C-w>s/q"
    );
    assert_eq!(group(&[Action::Window(W::New)]), "<C-w>n");
    assert_eq!(group(&[]), "");
  }

  #[test]
  fn placeholders_resolve() {
    let help = text();
    for (index, _) in help.match_indices('{') {
      let next = help[index + 1..].chars().next();
      assert!(!next.is_some_and(char::is_alphabetic), "{}", &help[index..]);
    }
  }

  #[test]
  fn reference_preserves_panel_and_operator_meanings() {
    let help = text();
    for line in [
      "P q: close focused panel",
      "gq: close sidebar from either focus",
      "o: swap selection ends",
      "P o: follow; keep panel focus",
      "P yy: yank row",
      "yy Y: join wrapped lines; rejoin hyphenated words",
      "V / y + i/a: inner / around objects",
      "( ) [ ] { } < >: paired brackets",
    ] {
      assert!(help.lines().any(|actual| actual == line), "{line}");
    }
  }
}
