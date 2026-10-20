use super::Form;
use smallvec::SmallVec;
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::{
  canonical_combining_class, decompose_compatible,
};

type Pending<T> = SmallVec<[(u8, char, T, T); 4]>;

pub(super) fn scan<T: Copy + Ord>(
  input: impl Iterator<Item = (char, T, T)>,
  form: Form,
  mut emit: impl FnMut(char, T, T),
) {
  let mut input = input.peekable();
  let mut pending = Pending::<T>::new();
  while let Some((ch, first, last)) = input.next() {
    if ch.is_ascii() && input.peek().is_none_or(|next| next.0.is_ascii()) {
      flush(&mut pending, form, &mut emit);
      emit(
        if form.folded() { ch.to_ascii_lowercase() } else { ch },
        first,
        last,
      );
      continue;
    }
    let mut push = |ch| {
      let class = canonical_combining_class(ch);
      if class == 0 {
        flush(&mut pending, form, &mut emit);
      }
      pending.push((class, ch, first, last));
    };
    decompose_compatible(ch, &mut push);
  }
  flush(&mut pending, form, &mut emit);
}

fn flush<T: Copy + Ord>(
  pending: &mut Pending<T>,
  form: Form,
  emit: &mut impl FnMut(char, T, T),
) {
  if pending.len() == 1 {
    let (_, ch, first, last) = pending.pop().expect("single character");
    single(ch, first, last, form, emit);
    return;
  }
  let Some(&(_, base, first, last)) = pending.first() else { return };
  let first = pending.iter().fold(first, |at, entry| at.min(entry.2));
  let last = pending.iter().fold(last, |at, entry| at.max(entry.3));
  let vowel =
    matches!(base.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y');
  if form.literal() && vowel && pending[1..].iter().all(|entry| tone(entry.1)) {
    let ch = if form.folded() { base.to_ascii_lowercase() } else { base };
    emit(ch, first, last);
    pending.clear();
    return;
  }
  let start = usize::from(pending[0].0 == 0);
  pending[start..].sort_by_key(|entry| entry.0);
  let mut write = |ch| {
    if form.literal() && vowel && tone(ch) {
      return;
    }
    emit(stroke(ch, form), first, last);
  };
  if form.folded() {
    let mut folded = SmallVec::<[char; 4]>::new();
    for &(_, ch, _, _) in pending.iter() {
      if ch.is_ascii() {
        folded.push(ch.to_ascii_lowercase());
      } else {
        folded.extend(ch.case_fold());
      }
    }
    folded.into_iter().nfkd().for_each(&mut write);
  } else {
    pending.iter().for_each(|entry| write(entry.1));
  }
  pending.clear();
}

fn single<T: Copy>(
  ch: char,
  first: T,
  last: T,
  form: Form,
  emit: &mut impl FnMut(char, T, T),
) {
  if !form.folded() || ch.is_ascii() {
    let ch = if form.folded() { ch.to_ascii_lowercase() } else { ch };
    emit(stroke(ch, form), first, last);
    return;
  }
  let mut folded = ch.case_fold();
  let head = folded.next().expect("case-folded character");
  if head == ch && folded.size_hint().1 == Some(0) {
    emit(stroke(ch, form), first, last);
    return;
  }
  for ch in std::iter::once(head).chain(folded).nfkd() {
    emit(stroke(ch, form), first, last);
  }
}

fn tone(ch: char) -> bool {
  matches!(
    ch,
    '\u{300}'
      | '\u{301}'
      | '\u{303}'
      | '\u{309}'
      | '\u{323}'
      | '\u{302}'
      | '\u{306}'
      | '\u{31b}'
  )
}

fn stroke(ch: char, form: Form) -> char {
  match (form.literal(), ch) {
    (true, 'đ') => 'd',
    (true, 'Đ') => 'D',
    _ => ch,
  }
}
