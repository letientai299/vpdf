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
  /// A chosen menu item or page label.
  Chosen,
  /// Text in a color the document chose, such as a highlight's.
  Swatch([u8; 3]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
  Black,
  White,
  BrightWhite,
  Blue,
  Magenta,
  Yellow,
  Cyan,
  Bar,
  ActiveBar,
  Error,
  PreviewBorder,
  PreviewText,
  Rgb([u8; 3]),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorChange {
  Keep,
  Default,
  Set(Tone),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attribute {
  Bold,
  Dim,
  Italic,
  Underline,
  Inverse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StylePatch {
  pub foreground: ColorChange,
  pub background: ColorChange,
  pub attributes: &'static [Attribute],
}

impl StylePatch {
  const fn new(
    foreground: ColorChange,
    background: ColorChange,
    attributes: &'static [Attribute],
  ) -> Self {
    Self { foreground, background, attributes }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleChange {
  Reset(StylePatch),
  Patch(StylePatch),
}

impl Role {
  pub const fn style(self) -> StyleChange {
    use Attribute::{Bold, Dim, Inverse, Italic, Underline};
    use ColorChange::{Keep, Set};
    use StyleChange::{Patch, Reset};
    use StylePatch as P;
    use Tone as T;
    match self {
      Self::Reset => Reset(P::new(Keep, Keep, &[])),
      Self::Bar | Self::TextCursorline => Reset(P::new(Keep, Set(T::Bar), &[])),
      Self::Selected => Reset(P::new(Keep, Set(T::Bar), &[Bold])),
      Self::StatusFile | Self::TabActive => {
        Reset(P::new(Set(T::Black), Set(T::ActiveBar), &[Bold]))
      }
      Self::TabInactive => Reset(P::new(Set(T::White), Set(T::Black), &[])),
      Self::Current => Reset(P::new(Set(T::Blue), Keep, &[Bold])),
      Self::Hover => Reset(P::new(Keep, Keep, &[Underline])),
      Self::HoverDim => Reset(P::new(Keep, Keep, &[Dim, Underline])),
      Self::BarHover => Reset(P::new(Keep, Set(T::Bar), &[Underline])),
      Self::Heading | Self::File => Patch(P::new(Set(T::Blue), Keep, &[Bold])),
      Self::Key => Patch(P::new(Set(T::Yellow), Keep, &[])),
      Self::Mode => Patch(P::new(Set(T::Cyan), Keep, &[])),
      Self::Argument => Patch(P::new(Set(T::Magenta), Keep, &[])),
      Self::Muted => Patch(P::new(Set(T::Bar), Keep, &[])),
      Self::Dim => Reset(P::new(Keep, Keep, &[Dim])),
      Self::StatusSection => Reset(P::new(Keep, Set(T::Bar), &[Italic])),
      Self::Error => Patch(P::new(Set(T::Error), Keep, &[Bold])),
      Self::TextLanding => Reset(P::new(Keep, Set(T::Yellow), &[])),
      Self::TextCursor => Reset(P::new(Set(T::Blue), Keep, &[Inverse])),
      Self::TextSelection => Reset(P::new(Set(T::Magenta), Keep, &[Inverse])),
      Self::TextMatch => Reset(P::new(Set(T::Yellow), Keep, &[Inverse])),
      Self::HintTyped => Reset(P::new(Set(T::Black), Set(T::Yellow), &[Bold])),
      Self::Hint => Reset(P::new(Set(T::Black), Set(T::Yellow), &[])),
      Self::HintPickedTyped => {
        Reset(P::new(Set(T::BrightWhite), Set(T::Blue), &[Bold]))
      }
      Self::HintPicked => Reset(P::new(Set(T::BrightWhite), Set(T::Blue), &[])),
      Self::TextLink => Reset(P::new(Set(T::Cyan), Keep, &[Underline])),
      Self::PreviewBorder => {
        Reset(P::new(Set(T::PreviewBorder), Set(T::BrightWhite), &[]))
      }
      Self::PreviewText => {
        Reset(P::new(Set(T::PreviewText), Set(T::BrightWhite), &[]))
      }
      Self::Badge(color) => Reset(P::new(
        Set(match color {
          Color::Blue => T::Blue,
          Color::Magenta => T::Magenta,
          Color::Yellow => T::Yellow,
          Color::Cyan => T::Cyan,
        }),
        Keep,
        &[Bold, Inverse],
      )),
      Self::Chosen => Self::Badge(Color::Blue).style(),
      Self::Swatch(rgb) => Patch(P::new(Set(T::Rgb(rgb)), Keep, &[])),
    }
  }
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
