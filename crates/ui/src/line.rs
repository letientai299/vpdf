//! Word boundaries for command-line editing, following zsh.
//!
//! zsh counts letters, digits, and `WORDCHARS` as word characters. Its
//! default `WORDCHARS` keeps paths whole for Alt-b/Alt-f; the narrower set
//! makes Ctrl-w and Ctrl-arrows stop at `/`, `.`, `:`, and `=`.

/// zsh's default `WORDCHARS`.
pub const SHELL: &str = "*?_-.[]~=/&;!#$%^(){}<>";
/// `WORDCHARS='_-'`, used by Ctrl-w and Ctrl-arrows.
pub const NARROW: &str = "_-";

fn word(ch: char, chars: &str) -> bool {
  ch.is_alphanumeric() || chars.contains(ch)
}

/// `backward-word`: start of the word before `cursor`.
pub fn word_start(text: &str, cursor: usize, chars: &str) -> usize {
  let head = &text[..cursor];
  let mut indices = head.char_indices().rev().peekable();
  let mut start = cursor;
  while let Some((index, ch)) = indices.peek().copied() {
    if word(ch, chars) {
      break;
    }
    start = index;
    indices.next();
  }
  for (index, ch) in indices {
    if !word(ch, chars) {
      break;
    }
    start = index;
  }
  start
}

/// `forward-word`: start of the next word after `cursor`.
pub fn next_word(text: &str, cursor: usize, chars: &str) -> usize {
  let tail = &text[cursor..];
  let skip = |from: usize, inside: bool| {
    tail[from..]
      .char_indices()
      .find(|(_, ch)| word(*ch, chars) != inside)
      .map_or(tail.len(), |(index, _)| from + index)
  };
  cursor + skip(skip(0, true), false)
}

/// `kill-word`: end of the word at or after `cursor`.
pub fn word_end(text: &str, cursor: usize, chars: &str) -> usize {
  let tail = &text[cursor..];
  let start = tail
    .char_indices()
    .find(|(_, ch)| word(*ch, chars))
    .map_or(tail.len(), |(index, _)| index);
  let end = tail[start..]
    .char_indices()
    .find(|(_, ch)| !word(*ch, chars))
    .map_or(tail.len(), |(index, _)| start + index);
  cursor + end
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn narrow_words_stop_at_punctuation() {
    let text = "e ~/papers/attention.pdf";
    assert_eq!(&text[word_start(text, text.len(), NARROW)..], "pdf");
    assert_eq!(
      &text[word_start(text, text.len(), SHELL)..],
      "~/papers/attention.pdf"
    );
    assert_eq!(word_start("page  12  ", 10, NARROW), 6);
    assert_eq!(next_word(text, 0, SHELL), 2);
    assert_eq!(word_end(text, 1, SHELL), text.len());
    assert_eq!(&text[..word_end(text, 4, NARROW)], "e ~/papers");
  }
}
