use crate::input::InputOwner;
use crate::prompt::PromptKind;
use vpdf_reader::reader::Mode;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Context {
  Normal,
  Cursor,
  Visual,
  Operator,
  Object,
  Resize,
  Register,
  MarkSet,
  MarkJump,
  Panel,
  Recent,
  HomeRecent,
  Hints,
  Region,
  Menu,
  Preview,
  Overview,
  Home,
  Prompt,
  Command,
  Search,
  Password,
}

impl Context {
  pub const ALL: &[Self] = &[
    Self::Normal,
    Self::Cursor,
    Self::Visual,
    Self::Operator,
    Self::Object,
    Self::Resize,
    Self::Register,
    Self::MarkSet,
    Self::MarkJump,
    Self::Panel,
    Self::Recent,
    Self::HomeRecent,
    Self::Hints,
    Self::Region,
    Self::Menu,
    Self::Preview,
    Self::Overview,
    Self::Home,
    Self::Prompt,
    Self::Command,
    Self::Search,
    Self::Password,
  ];

  pub const fn name(self) -> &'static str {
    match self {
      Self::Normal => "normal",
      Self::Cursor => "cursor",
      Self::Visual => "visual",
      Self::Operator => "operator",
      Self::Object => "object",
      Self::Resize => "resize",
      Self::Register => "register",
      Self::MarkSet => "mark-set",
      Self::MarkJump => "mark-jump",
      Self::Panel => "panel",
      Self::Recent => "recent",
      Self::HomeRecent => "home-recent",
      Self::Hints => "hints",
      Self::Region => "region",
      Self::Menu => "menu",
      Self::Preview => "preview",
      Self::Overview => "overview",
      Self::Home => "home",
      Self::Prompt => "prompt",
      Self::Command => "command",
      Self::Search => "search",
      Self::Password => "password",
    }
  }

  pub fn parse(name: &str) -> Option<Self> {
    Self::ALL.iter().copied().find(|context| context.name() == name)
  }

  pub fn resolve(owner: InputOwner, mode: Mode, operator: bool) -> Self {
    match owner {
      InputOwner::Prompt => Self::Prompt,
      InputOwner::Hints => Self::Hints,
      InputOwner::Region => Self::Region,
      InputOwner::Context => Self::Menu,
      InputOwner::Peek => Self::Preview,
      InputOwner::Panel => Self::Panel,
      InputOwner::Overview => Self::Overview,
      InputOwner::Content if operator => Self::Operator,
      InputOwner::Content => match mode {
        Mode::Normal => Self::Normal,
        Mode::Cursor => Self::Cursor,
        Mode::Visual(_) => Self::Visual,
      },
    }
  }

  pub const fn prompt(kind: PromptKind) -> Self {
    match kind {
      PromptKind::Command => Self::Command,
      PromptKind::Search(_) => Self::Search,
      PromptKind::Password => Self::Password,
    }
  }

  pub const fn parent(self) -> Option<Self> {
    match self {
      Self::Command | Self::Search | Self::Password => Some(Self::Prompt),
      Self::Recent => Some(Self::Panel),
      Self::HomeRecent => Some(Self::Home),
      _ => None,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_reader::reader::SelectionKind;

  #[test]
  fn owners_isolate_input_modes() {
    for owner in [
      InputOwner::Panel,
      InputOwner::Context,
      InputOwner::Region,
      InputOwner::Hints,
      InputOwner::Peek,
      InputOwner::Overview,
    ] {
      let context = Context::resolve(owner, Mode::Normal, false);
      assert_eq!(
        Context::resolve(owner, Mode::Visual(SelectionKind::Line), true),
        context
      );
    }
    assert_eq!(
      Context::resolve(InputOwner::Content, Mode::Normal, false),
      Context::Normal
    );
    assert_eq!(
      Context::resolve(InputOwner::Content, Mode::Cursor, false),
      Context::Cursor
    );
    assert_eq!(
      Context::resolve(
        InputOwner::Content,
        Mode::Visual(SelectionKind::Line),
        false
      ),
      Context::Visual
    );
    assert_eq!(
      Context::resolve(InputOwner::Content, Mode::Cursor, true),
      Context::Operator
    );
    for kind in
      [PromptKind::Command, PromptKind::Password, PromptKind::Search(false)]
    {
      assert_eq!(Context::prompt(kind).parent(), Some(Context::Prompt));
    }
    for &context in Context::ALL {
      assert_eq!(Context::parse(context.name()), Some(context));
    }
    assert_eq!(Context::parse("unknown"), None);
  }
}
