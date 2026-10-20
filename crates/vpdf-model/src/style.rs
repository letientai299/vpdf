#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
  Blue = 34,
  Magenta = 35,
  Yellow = 33,
  Cyan = 36,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
  Reset,
  Bar,
  Selected,
  StatusFile,
  TabActive,
  TabInactive,
  Current,
  Hover,
  HoverDim,
  BarHover,
  Heading,
  Key,
  Mode,
  Argument,
  File,
  Muted,
  Dim,
  StatusSection,
  Error,
  TextLanding,
  TextCursorline,
  TextCursor,
  TextSelection,
  TextMatch,
  HintTyped,
  Hint,
  HintPickedTyped,
  HintPicked,
  TextLink,
  PreviewBorder,
  PreviewText,
  Badge(Color),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Shape {
  #[default]
  Default,
  Pointer,
  Text,
  Crosshair,
  Grabbing,
  ColResize,
  RowResize,
}
