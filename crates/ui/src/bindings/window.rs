use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AxisArg {
  Rows,
  Columns,
}
impl From<AxisArg> for crate::split::Axis {
  fn from(value: AxisArg) -> Self {
    match value {
      AxisArg::Rows => Self::Rows,
      AxisArg::Columns => Self::Columns,
    }
  }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DirectionArg {
  Left,
  Down,
  Up,
  Right,
}
impl From<DirectionArg> for crate::split::Direction {
  fn from(value: DirectionArg) -> Self {
    match value {
      DirectionArg::Left => Self::Left,
      DirectionArg::Down => Self::Down,
      DirectionArg::Up => Self::Up,
      DirectionArg::Right => Self::Right,
    }
  }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "kebab-case", deny_unknown_fields)]
pub enum WindowArg {
  Split { axis: AxisArg },
  Focus { direction: DirectionArg },
  Next,
  Previous,
  Last,
  Close,
  Only,
  Equalize,
  Resize { axis: AxisArg, amount: i32 },
  Extend { direction: DirectionArg },
  Maximize { axis: AxisArg },
  Exchange,
  Rotate { forward: bool },
  New,
  Move { direction: DirectionArg },
  ToTab,
}
impl From<WindowArg> for crate::window::Window {
  fn from(value: WindowArg) -> Self {
    match value {
      WindowArg::Split { axis } => Self::Split(axis.into()),
      WindowArg::Focus { direction } => Self::Focus(direction.into()),
      WindowArg::Next => Self::Next,
      WindowArg::Previous => Self::Previous,
      WindowArg::Last => Self::Last,
      WindowArg::Close => Self::Close,
      WindowArg::Only => Self::Only,
      WindowArg::Equalize => Self::Equalize,
      WindowArg::Resize { axis, amount } => Self::Resize(axis.into(), amount),
      WindowArg::Extend { direction } => Self::Extend(direction.into()),
      WindowArg::Maximize { axis } => Self::Maximize(axis.into()),
      WindowArg::Exchange => Self::Exchange,
      WindowArg::Rotate { forward } => Self::Rotate(forward),
      WindowArg::New => Self::New,
      WindowArg::Move { direction } => Self::Move(direction.into()),
      WindowArg::ToTab => Self::ToTab,
    }
  }
}

impl From<crate::window::Window> for WindowArg {
  fn from(value: crate::window::Window) -> Self {
    use crate::window::Window as W;
    match value {
      W::Split(axis) => Self::Split { axis: axis.into() },
      W::Focus(direction) => Self::Focus { direction: direction.into() },
      W::Next => Self::Next,
      W::Previous => Self::Previous,
      W::Last => Self::Last,
      W::Close => Self::Close,
      W::Only => Self::Only,
      W::Equalize => Self::Equalize,
      W::Resize(axis, amount) => Self::Resize { axis: axis.into(), amount },
      W::Extend(direction) => Self::Extend { direction: direction.into() },
      W::Maximize(axis) => Self::Maximize { axis: axis.into() },
      W::Exchange => Self::Exchange,
      W::Rotate(forward) => Self::Rotate { forward },
      W::New => Self::New,
      W::Move(direction) => Self::Move { direction: direction.into() },
      W::ToTab => Self::ToTab,
    }
  }
}
impl From<crate::split::Axis> for AxisArg {
  fn from(value: crate::split::Axis) -> Self {
    match value {
      crate::split::Axis::Rows => Self::Rows,
      crate::split::Axis::Columns => Self::Columns,
    }
  }
}
impl From<crate::split::Direction> for DirectionArg {
  fn from(value: crate::split::Direction) -> Self {
    match value {
      crate::split::Direction::Left => Self::Left,
      crate::split::Direction::Right => Self::Right,
      crate::split::Direction::Up => Self::Up,
      crate::split::Direction::Down => Self::Down,
    }
  }
}
