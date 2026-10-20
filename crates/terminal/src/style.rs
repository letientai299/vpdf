use vpdf_model::style::{Color, Role};

pub const fn sgr(role: Role) -> &'static str {
  match role {
    Role::Reset => "\x1b[0m",
    Role::Bar | Role::TextCursorline => "\x1b[0;100m",
    Role::Selected => "\x1b[0;1;100m",
    Role::StatusFile | Role::TabActive => "\x1b[0;1;30;104m",
    Role::TabInactive => "\x1b[0;37;40m",
    Role::Current => "\x1b[0;1;34m",
    Role::Hover => "\x1b[0;4m",
    Role::HoverDim => "\x1b[0;2;4m",
    Role::BarHover => "\x1b[0;4;100m",
    Role::Heading | Role::File => "\x1b[1;34m",
    Role::Key => "\x1b[33m",
    Role::Mode => "\x1b[36m",
    Role::Argument => "\x1b[35m",
    Role::Muted => "\x1b[90m",
    Role::Dim => "\x1b[0;2m",
    Role::StatusSection => "\x1b[0;3;100m",
    Role::Error => "\x1b[1;31m",
    Role::TextLanding => "\x1b[0;43m",
    Role::TextCursor => "\x1b[0;7;34m",
    Role::TextSelection => "\x1b[0;7;35m",
    Role::TextMatch => "\x1b[0;7;33m",
    Role::HintTyped => "\x1b[0;1;30;43m",
    Role::Hint => "\x1b[0;30;43m",
    Role::HintPickedTyped => "\x1b[0;1;97;44m",
    Role::HintPicked => "\x1b[0;97;44m",
    Role::TextLink => "\x1b[0;4;36m",
    Role::PreviewBorder => "\x1b[0;38;2;60;120;230;48;2;255;255;255m",
    Role::PreviewText => "\x1b[0;38;2;20;20;20;48;2;255;255;255m",
    Role::Badge(Color::Blue) => "\x1b[0;1;7;34m",
    Role::Badge(Color::Magenta) => "\x1b[0;1;7;35m",
    Role::Badge(Color::Yellow) => "\x1b[0;1;7;33m",
    Role::Badge(Color::Cyan) => "\x1b[0;1;7;36m",
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use sha2::{Digest, Sha256};

  #[test]
  fn palette_keeps_legacy_bytes() {
    let roles = [
      Role::Reset,
      Role::Bar,
      Role::Selected,
      Role::StatusFile,
      Role::TabActive,
      Role::TabInactive,
      Role::Current,
      Role::Hover,
      Role::HoverDim,
      Role::BarHover,
      Role::Heading,
      Role::Key,
      Role::Mode,
      Role::Argument,
      Role::File,
      Role::Muted,
      Role::Dim,
      Role::StatusSection,
      Role::Error,
      Role::TextLanding,
      Role::TextCursorline,
      Role::TextCursor,
      Role::TextSelection,
      Role::TextMatch,
      Role::HintTyped,
      Role::Hint,
      Role::HintPickedTyped,
      Role::HintPicked,
      Role::TextLink,
      Role::PreviewBorder,
      Role::PreviewText,
      Role::Badge(Color::Blue),
      Role::Badge(Color::Magenta),
      Role::Badge(Color::Yellow),
      Role::Badge(Color::Cyan),
    ];
    let mut hash = Sha256::new();
    for role in roles {
      hash.update(sgr(role).as_bytes());
    }
    // Original palette, in declaration order, then badges.
    let expected: [u8; 32] = [
      0x8c, 0x78, 0xa1, 0x3f, 0xd4, 0xa4, 0x78, 0xed, 0x03, 0x4e, 0xeb, 0x5f,
      0x88, 0xe1, 0xcd, 0x07, 0x6f, 0xcd, 0x63, 0x40, 0x01, 0xe8, 0x7a, 0x4f,
      0x35, 0x1f, 0xef, 0x86, 0xae, 0x47, 0xd4, 0xb0,
    ];
    assert_eq!(hash.finalize().as_slice(), expected.as_slice());
  }
}
