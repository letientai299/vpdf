use super::{
  Action, BINDINGS, Grammar, Input, KeyCode, KeyEvent, KeyModifiers, plain,
  token,
};
use crate::bindings::{
  Bindings, Context, Control, Map, Match, Pending, Sequence, Target,
};
use std::sync::Arc;
use vpdf_reader::motion::Motion;

#[derive(Clone, Copy, Debug, Default)]
enum Capture {
  #[default]
  None,
  Register,
  SetMark,
  GoMark(bool),
  Object(bool),
  Operator {
    panel: bool,
  },
  Resize,
}

#[derive(Clone, Default)]
pub struct Keys {
  prefix: String,
  count: usize,
  register: Option<char>,
  capture: Capture,
  sequence: Pending,
  cancellation: Pending,
  map: Option<Arc<Map<Target>>>,
  cancelled: bool,
}

#[derive(Clone, Debug)]
pub enum Dispatch {
  Input(Input),
  Command(String),
}

impl Keys {
  pub(crate) fn references(
    context: Context,
    bindings: &Bindings,
  ) -> Vec<crate::bindings::Binding> {
    let mut rows = Vec::new();
    Self::default().reference_walk(context, bindings, &[], &[], &mut rows);
    rows
  }

  fn reference_walk(
    &self,
    context: Context,
    bindings: &Bindings,
    prefix: &[KeyEvent],
    sources: &[crate::reference::Source],
    rows: &mut Vec<crate::bindings::Binding>,
  ) {
    let map = self.map.clone().unwrap_or_else(|| bindings.map(context));
    let mut entries: Vec<_> = map
      .entries()
      .map(|(key, target)| (key.clone(), Some(target.clone())))
      .collect();
    if matches!(self.capture, Capture::SetMark | Capture::GoMark(_)) {
      entries.extend(('a'..='z').chain('A'..='Z').filter_map(|ch| {
        let key = Sequence::parse(&ch.to_string()).ok()?;
        map.get(&key).is_none().then_some((key, None))
      }));
    }
    for (sequence, target) in entries {
      if !prefix.is_empty() && target.as_ref().is_some_and(Target::cancels) {
        continue;
      }
      let mut state = self.clone();
      let mut dispatch = None;
      for &key in sequence.keys() {
        dispatch = state.feed_with(key, context, bindings);
      }
      let mut keys = prefix.to_vec();
      keys.extend_from_slice(sequence.keys());
      let mut origins = sources.to_vec();
      let source = map.source(&sequence);
      if !origins.contains(&source) {
        origins.push(source);
      }
      let resolved = match dispatch {
        Some(Dispatch::Input(input)) => Some(Target::Reader(input.action)),
        Some(Dispatch::Command(command)) => Some(Target::Command(command)),
        None => target.filter(|target| {
          matches!(
            target,
            Target::Prompt(_) | Target::Hint(_) | Target::Control(_)
          )
        }),
      };
      if let Some(target) = resolved {
        rows.push(crate::bindings::Binding {
          context,
          sequence: Sequence::from_keys(&keys).expect("resolved binding"),
          purpose: target.about(),
          target,
          sources: origins.clone(),
        });
      }
      if matches!(
        state.capture,
        Capture::SetMark
          | Capture::GoMark(_)
          | Capture::Object(_)
          | Capture::Operator { .. }
          | Capture::Resize
      ) && std::mem::discriminant(&self.capture)
        != std::mem::discriminant(&state.capture)
      {
        state.reference_walk(context, bindings, &keys, &origins, rows);
      }
    }
  }

  pub fn clear(&mut self) {
    *self = Self::default();
  }

  pub fn feed(&mut self, key: KeyEvent, grammar: Grammar) -> Option<Input> {
    let context = match grammar {
      Grammar::Normal => Context::Normal,
      Grammar::Cursor | Grammar::Other => Context::Cursor,
      Grammar::Objects => Context::Visual,
      Grammar::Panel => Context::Panel,
    };
    match self.feed_with(key, context, &Bindings::default())? {
      Dispatch::Input(input) => {
        if grammar == Grammar::Other && input.action == Action::Yank {
          self.clear();
        }
        Some(input)
      }
      Dispatch::Command(_) => None,
    }
  }

  pub fn feed_with(
    &mut self,
    key: KeyEvent,
    context: Context,
    bindings: &Bindings,
  ) -> Option<Dispatch> {
    if key.kind == vpdf_model::input::KeyEventKind::Release {
      return None;
    }
    let context = match self.capture {
      Capture::Register => Context::Register,
      Capture::SetMark => Context::MarkSet,
      Capture::GoMark(_) => Context::MarkJump,
      Capture::Object(_) => Context::Object,
      Capture::Operator { .. } => Context::Operator,
      Capture::Resize => Context::Resize,
      Capture::None => context,
    };
    let map = bindings.map(context);
    self.map = Some(Arc::clone(&map));
    if matches!(
      self.capture,
      Capture::Register | Capture::SetMark | Capture::GoMark(_)
    ) {
      return self.capture_key(key, &map);
    }
    let mut typed = self.sequence.keys().to_vec();
    typed.push(key);
    match self.sequence.feed_cancellable(&map, key, Target::cancels) {
      Match::Target(target) => {
        self.target(target, context, bindings, show(&typed))
      }
      Match::Pending => {
        self.prefix = show(self.sequence.keys());
        None
      }
      Match::Consumed | Match::Unbound => {
        if !self.resizing() {
          self.clear();
        }
        None
      }
    }
  }

  fn capture_key(
    &mut self,
    key: KeyEvent,
    map: &Map<Target>,
  ) -> Option<Dispatch> {
    match self.cancellation.feed_cancellable(map, key, Target::cancels) {
      Match::Target(Target::Reader(Action::Cancel)) => {
        self.cancel();
        return Some(Dispatch::Input(Input::plain(Action::Cancel)));
      }
      Match::Target(Target::Reader(action)) => {
        return Some(Dispatch::Input(self.finish(action)));
      }
      Match::Target(_) | Match::Pending | Match::Consumed => return None,
      Match::Unbound => {}
    }
    let capture = self.capture;
    self.capture = Capture::None;
    self.prefix.clear();
    let KeyCode::Char(mut name) = key.code else {
      self.clear();
      return None;
    };
    if !plain(key) {
      self.clear();
      return None;
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
      name.make_ascii_uppercase();
    }
    match capture {
      Capture::Register => {
        self.register = Some(name);
        None
      }
      Capture::SetMark if name.is_ascii_alphabetic() => {
        Some(Dispatch::Input(self.finish(Action::Mark(name))))
      }
      Capture::GoMark(exact) if name.is_ascii_alphabetic() => {
        Some(Dispatch::Input(self.finish(Action::GoMark(name, exact))))
      }
      _ => {
        self.clear();
        None
      }
    }
  }

  fn target(
    &mut self,
    target: Target,
    context: Context,
    bindings: &Bindings,
    typed: String,
  ) -> Option<Dispatch> {
    match target {
      Target::Control(Control::Digit { digit }) => {
        if digit == 0 && self.count == 0 {
          if matches!(self.capture, Capture::Object(_) | Capture::Resize) {
            self.clear();
            return None;
          }
          return Some(Dispatch::Input(
            self.finish(Action::Motion(Motion::LineStart)),
          ));
        }
        self.count = self
          .count
          .saturating_mul(10)
          .saturating_add(usize::from(digit))
          .min(10_000);
        None
      }
      Target::Control(control) => {
        self.prefix = typed;
        self.capture = match control {
          Control::Register => Capture::Register,
          Control::SetMark => Capture::SetMark,
          Control::GoMark { exact } => Capture::GoMark(exact),
          Control::Object { around } => Capture::Object(around),
          Control::Resize => Capture::Resize,
          Control::Yank => Capture::Operator {
            panel: matches!(context, Context::Panel | Context::Recent),
          },
          Control::Digit { .. } => unreachable!(),
        };
        self.map = Some(bindings.map(match self.capture {
          Capture::Register => Context::Register,
          Capture::SetMark => Context::MarkSet,
          Capture::GoMark(_) => Context::MarkJump,
          Capture::Object(_) => Context::Object,
          Capture::Operator { .. } => Context::Operator,
          Capture::Resize => Context::Resize,
          Capture::None => context,
        }));
        None
      }
      Target::Reader(Action::Cancel) => {
        self.cancel();
        Some(Dispatch::Input(Input::plain(Action::Cancel)))
      }
      Target::Reader(action) => {
        let register = self.register;
        let resizing = self.resizing();
        let prefix = self.prefix.clone();
        let action =
          if matches!(self.capture, Capture::Operator { panel: true })
            && action == Action::Yank
          {
            Action::YankLine
          } else {
            action
          };
        let input = self.finish(action);
        if resizing {
          self.capture = Capture::Resize;
          self.prefix = prefix;
          self.map = Some(bindings.map(Context::Resize));
        } else if action == Action::Yank
          && matches!(context, Context::Normal | Context::Cursor)
        {
          self.capture = Capture::Operator { panel: false };
          self.prefix = typed;
          self.register = register;
          self.map = Some(bindings.map(Context::Operator));
        }
        Some(Dispatch::Input(input))
      }
      Target::TextObject(object) => {
        let around = matches!(self.capture, Capture::Object(true));
        Some(Dispatch::Input(self.finish(Action::Object(object, around))))
      }
      Target::Command(command) => {
        self.clear();
        Some(Dispatch::Command(command))
      }
      Target::Prompt(_) | Target::Hint(_) => {
        self.clear();
        None
      }
    }
  }

  fn cancel(&mut self) {
    let pending = !self.pending().is_empty();
    self.clear();
    self.cancelled = pending;
  }

  pub fn cancelled_pending(&self) -> bool {
    self.cancelled
  }

  pub fn resizing(&self) -> bool {
    matches!(self.capture, Capture::Resize)
  }

  pub fn awaiting(&self) -> Option<&str> {
    (!self.prefix.is_empty()).then_some(&self.prefix)
  }

  pub fn key_for(&self, action: Action) -> Option<String> {
    let map = self.map.as_ref()?;
    let current = Sequence::parse(&self.prefix).ok();
    if let Some(current) = current
      && map.get(&current) == Some(&Target::Reader(action))
    {
      return Some(current.to_string());
    }
    map
      .entries()
      .find(|(_, target)| **target == Target::Reader(action))
      .map(|(sequence, _)| sequence.to_string())
  }

  pub fn choosing(&self) -> Option<Control> {
    match self.capture {
      Capture::Register => Some(Control::Register),
      Capture::SetMark => Some(Control::SetMark),
      Capture::GoMark(exact) => Some(Control::GoMark { exact }),
      _ => None,
    }
  }

  pub fn pending(&self) -> String {
    let mut text = String::new();
    if let Some(register) = self.register {
      text.push('"');
      text.push(register);
    }
    if self.count > 0 {
      text += &self.count.to_string();
    }
    text + &self.prefix
  }

  fn finish(&mut self, action: Action) -> Input {
    let input = Input {
      action,
      count: self.count.max(1),
      explicit: self.count > 0,
      register: self.register.unwrap_or('"'),
    };
    self.clear();
    input
  }

  fn operator_clue(&self, target: &Target) -> bool {
    let Capture::Operator { panel } = self.capture else { return true };
    match target {
      Target::Reader(
        Action::Hints(true) | Action::YankPath | Action::YankRaw,
      )
      | Target::Control(Control::Object { .. })
      | Target::TextObject(_)
      | Target::Command(_) => true,
      Target::Reader(Action::Yank) => panel,
      _ => false,
    }
  }

  pub fn effective_clues(
    &self,
    allows: impl Fn(Action) -> bool,
  ) -> Vec<(Sequence, String)> {
    if self.awaiting().is_none() || self.choosing().is_some() {
      return Vec::new();
    }
    let Some(map) = &self.map else { return Vec::new() };
    map
      .entries()
      .filter_map(|(sequence, target)| {
        if !sequence.starts_with(self.sequence.keys()) || target.cancels() {
          return None;
        }
        let suffix =
          Sequence::from_keys(&sequence.keys()[self.sequence.keys().len()..])
            .ok()?;
        let action = match target {
          Target::Reader(action) => Some(*action),
          Target::TextObject(object) => Some(Action::Object(
            *object,
            matches!(self.capture, Capture::Object(true)),
          )),
          Target::Control(Control::Object { around }) => {
            Some(Action::Object(vpdf_reader::objects::Object::Word, *around))
          }
          Target::Control(Control::SetMark) => Some(Action::Mark('a')),
          Target::Control(Control::GoMark { exact }) => {
            Some(Action::GoMark('a', *exact))
          }
          _ => None,
        };
        if action.is_some_and(|action| !allows(action)) {
          return None;
        }
        if !self.operator_clue(target) {
          return None;
        }
        let about = match target {
          Target::Command(command) => format!(":{command}"),
          Target::Control(Control::Object { around }) => {
            if *around {
              "around object".into()
            } else {
              "inner object".into()
            }
          }
          Target::Control(Control::Digit { digit }) => {
            format!("count digit {digit}")
          }
          Target::Control(Control::Register) => "choose register".into(),
          Target::Control(Control::SetMark) => "choose mark".into(),
          Target::Control(Control::GoMark { .. }) => "jump to mark".into(),
          Target::Control(Control::Resize) => "resize panes".into(),
          Target::Control(Control::Yank) => "yank row".into(),
          _ => action
            .and_then(|action| {
              BINDINGS
                .iter()
                .find(|(_, bound, _)| *bound == action)
                .map(|(_, _, about)| (*about).to_owned())
            })
            .unwrap_or_else(|| target.label()),
        };
        Some((suffix, about))
      })
      .collect()
  }

  pub fn clues(
    &self,
    _grammar: Grammar,
    allows: impl Fn(Action) -> bool,
  ) -> Vec<(String, &'static str)> {
    if matches!(
      self.capture,
      Capture::Register | Capture::SetMark | Capture::GoMark(_)
    ) {
      return Vec::new();
    }
    if self.awaiting().is_none() {
      return Vec::new();
    }
    let Some(map) = &self.map else { return Vec::new() };
    let mut clues: Vec<_> = map
      .entries()
      .filter_map(|(sequence, target)| {
        if !sequence.starts_with(self.sequence.keys()) {
          return None;
        }
        let suffix = &sequence.keys()[self.sequence.keys().len()..];
        if suffix.is_empty() {
          return None;
        }
        if target.cancels() {
          return None;
        }
        if !self.operator_clue(target) {
          return None;
        }
        let action = match target {
          Target::Reader(action) => *action,
          Target::TextObject(object) => Action::Object(
            *object,
            matches!(self.capture, Capture::Object(true)),
          ),
          _ => return None,
        };
        if !allows(action) {
          return None;
        }
        let about = BINDINGS
          .iter()
          .find(|(_, bound, _)| *bound == action)
          .map_or("action", |(_, _, about)| *about);
        Some((show(suffix), about))
      })
      .collect();
    clues.sort_by(|left, right| left.0.cmp(&right.0));
    clues.dedup();
    clues
  }
}

fn show(keys: &[KeyEvent]) -> String {
  keys
    .iter()
    .map(|&key| {
      let key = Sequence::from_keys(&[key]).ok().map_or(key, |s| s.keys()[0]);
      match key.code {
        KeyCode::Left
        | KeyCode::Right
        | KeyCode::Up
        | KeyCode::Down
        | KeyCode::Home
        | KeyCode::End
        | KeyCode::PageUp
        | KeyCode::PageDown
        | KeyCode::F(_) => Sequence::from_keys(&[key])
          .map_or_else(|_| String::new(), |sequence| sequence.to_string()),
        _ => token(key).unwrap_or_else(|| {
          Sequence::from_keys(&[key])
            .map_or_else(|_| String::new(), |sequence| sequence.to_string())
        }),
      }
    })
    .collect()
}
