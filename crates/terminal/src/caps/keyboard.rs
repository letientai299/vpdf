use super::{Caps, Host, Reply, Result, flags, fmt, is};

/// Preserve text and text-producing key modifiers.
pub(super) const KEYBOARD: u32 =
  DISAMBIGUATE | ALTERNATE_KEYS | ALL_KEYS | TEXT;

/// The required flag: without it Esc, Ctrl-i, and Alt chords share
/// bytes with other keys.
pub(super) const DISAMBIGUATE: u32 = 1;

pub(super) const ALTERNATE_KEYS: u32 = 4;
const ALL_KEYS: u32 = 8;
const TEXT: u32 = 16;

/// xterm modifyOtherKeys mode 2, which tmux turns into extended keys.
pub(super) const MODIFY_OTHER_KEYS: &[u8] = b"\x1b[>4;2m";

/// Rounds that read the flags back before the reader gives up; a
/// refusing terminal answers each at once.
pub(super) const READBACKS: usize = 3;

/// Why keys cannot be told apart.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyLack {
  /// GNU screen, asked nothing.
  Screen,
  /// DA1 missed `WAIT`, so the probe cannot tell.
  Unanswered,
  /// No answer to `CSI ? u`; the XTVERSION of what answered, if any.
  Protocol(Option<String>),
  /// Flags in effect after pushing `KEYBOARD`; None without an answer.
  Refused(Option<u32>),
  /// tmux with `extended-keys off`, so Ctrl chords arrive as legacy
  /// bytes.
  ExtendedKeys,
  /// tmux before 3.2, which has no extended keys.
  TmuxRelease,
}

impl fmt::Display for KeyLack {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match self {
      Self::Screen => {
        write!(f, "GNU screen lacks Kitty keyboard disambiguation")
      }
      Self::Unanswered => write!(f, "Terminal did not answer queries"),
      Self::Protocol(name) if is(name.as_deref(), &["wezterm"]) => {
        write!(f, "WezTerm needs enable_kitty_keyboard = true")
      }
      Self::Protocol(name) => {
        let name = name.as_deref().unwrap_or("Terminal");
        write!(f, "{name} lacks Kitty keyboard disambiguation")
      }
      Self::Refused(Some(flags)) => {
        write!(f, "Kitty keyboard disambiguation refused, flags {flags}")
      }
      Self::Refused(None) => {
        write!(f, "Terminal did not confirm Kitty keyboard flags")
      }
      Self::ExtendedKeys => {
        write!(f, "tmux needs `set -s extended-keys on` in tmux.conf")
      }
      Self::TmuxRelease => write!(f, "tmux before 3.2 lacks extended keys"),
    }
  }
}

impl std::error::Error for KeyLack {}

impl Caps {
  /// Pushes the reader's keyboard flags where the protocol answered and
  /// reads back the flags in effect in the same round; `ask` sends a
  /// request and returns the replies up to DA1's. None once keys are
  /// told apart.
  pub fn negotiate(
    &self,
    mut ask: impl FnMut(&[u8]) -> Result<Vec<Reply>>,
  ) -> Result<Option<KeyLack>> {
    // `Lack::Tmux` already stops the start.
    if self.host == Host::Tmux(None) {
      return Ok(None);
    }
    if let Host::Tmux(Some(tmux)) = &self.host {
      // tmux never answers `CSI ? u`; with `extended-keys` it sends Ctrl
      // chords as `CSI 27 ; m ; k ~` or `CSI k ; m u` once asked for
      // modifyOtherKeys mode 2, whichever `extended-keys-format` says.
      if tmux.before(&[3, 2]) {
        return Ok(Some(KeyLack::TmuxRelease));
      }
      if !tmux.extended_keys {
        return Ok(Some(KeyLack::ExtendedKeys));
      }
      ask(MODIFY_OTHER_KEYS)?;
      return Ok(None);
    }
    if self.keyboard.is_none() {
      return Ok(self.key_lack(None));
    }
    let push = format!("\x1b[>{KEYBOARD}u\x1b[?u");
    let mut request = push.as_bytes();
    let mut enabled = None;
    // Answers a child program left unread, such as its own flags and
    // DA1, can end a round before the readback; each later round
    // settles one stale DA1 and collects what came late. The last
    // flags are the readback.
    for _ in 0..READBACKS {
      let replies = ask(request)?;
      enabled = replies.iter().rev().find_map(flags).or(enabled);
      if enabled.is_some_and(|flags| flags & DISAMBIGUATE != 0) {
        break;
      }
      request = b"\x1b[?u";
    }
    Ok(self.key_lack(enabled))
  }
}

impl Caps {
  /// Why keys stay ambiguous given `enabled`, the flags in effect after
  /// pushing `KEYBOARD`.
  pub(super) fn key_lack(&self, enabled: Option<u32>) -> Option<KeyLack> {
    if self.host == Host::Screen {
      return Some(KeyLack::Screen);
    }
    if self.keyboard.is_none() {
      return Some(match self.attributes {
        None => KeyLack::Unanswered,
        Some(_) => KeyLack::Protocol(self.version.clone()),
      });
    }
    match enabled {
      Some(flags) if flags & DISAMBIGUATE != 0 => None,
      enabled => Some(KeyLack::Refused(enabled)),
    }
  }
}
