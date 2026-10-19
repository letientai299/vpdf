//! Installed fonts for PDFs that name a font without embedding it.
//!
//! The binary carries only `MuPDF`'s URW base14 fonts, so base14 names keep
//! them. Other names resolve through Core Text on macOS and `fc-match`
//! elsewhere; spawning `fc-match` keeps fontconfig an optional runtime
//! tool for the static musl build. On a miss `MuPDF` substitutes a base14
//! font, and CJK text without an installed font draws wrong glyphs.

use mupdf::{CjkFontOrdering, Font, FontHints, FontLoader};
use std::sync::Once;

/// Routes `MuPDF`'s lookups for unembedded fonts here.
pub fn install() {
  static ONCE: Once = Once::new();
  ONCE.call_once(|| mupdf::set_font_loader(Installed));
}

struct Installed;

impl FontLoader for Installed {
  fn load_font(&self, name: &str, hints: FontHints) -> Option<Font> {
    let name = unsubset(name);
    if is_base14(name) {
      return None;
    }
    system::named(name, hints)
  }

  fn load_cjk_font(
    &self,
    name: &str,
    ordering: CjkFontOrdering,
    serif: bool,
  ) -> Option<Font> {
    let name = unsubset(name);
    let exact = (!name.is_empty())
      .then(|| system::named(name, FontHints::default()))
      .flatten();
    exact.or_else(|| system::cjk(ordering, serif))
  }
}

/// `MuPDF`'s base14 aliases, compared without spaces.
const BASE14: &[&str] = &[
  "Courier",
  "CourierNew",
  "CourierNewPSMT",
  "Courier-Bold",
  "CourierNew,Bold",
  "Courier,Bold",
  "CourierNewPS-BoldMT",
  "CourierNew-Bold",
  "Courier-Oblique",
  "CourierNew,Italic",
  "Courier,Italic",
  "CourierNewPS-ItalicMT",
  "CourierNew-Italic",
  "Courier-BoldOblique",
  "CourierNew,BoldItalic",
  "Courier,BoldItalic",
  "CourierNewPS-BoldItalicMT",
  "CourierNew-BoldItalic",
  "Helvetica",
  "ArialMT",
  "Arial",
  "Helvetica-Bold",
  "Arial-BoldMT",
  "Arial,Bold",
  "Arial-Bold",
  "Helvetica,Bold",
  "Helvetica-Oblique",
  "Arial-ItalicMT",
  "Arial,Italic",
  "Arial-Italic",
  "Helvetica,Italic",
  "Helvetica-Italic",
  "Helvetica-BoldOblique",
  "Arial-BoldItalicMT",
  "Arial,BoldItalic",
  "Arial-BoldItalic",
  "Helvetica,BoldItalic",
  "Helvetica-BoldItalic",
  "Times-Roman",
  "TimesNewRomanPSMT",
  "TimesNewRoman",
  "TimesNewRomanPS",
  "Times-Bold",
  "TimesNewRomanPS-BoldMT",
  "TimesNewRoman,Bold",
  "TimesNewRomanPS-Bold",
  "TimesNewRoman-Bold",
  "Times-Italic",
  "TimesNewRomanPS-ItalicMT",
  "TimesNewRoman,Italic",
  "TimesNewRomanPS-Italic",
  "TimesNewRoman-Italic",
  "Times-BoldItalic",
  "TimesNewRomanPS-BoldItalicMT",
  "TimesNewRoman,BoldItalic",
  "TimesNewRomanPS-BoldItalic",
  "TimesNewRoman-BoldItalic",
  "Symbol",
  "Symbol,Italic",
  "Symbol,Bold",
  "Symbol,BoldItalic",
  "SymbolMT",
  "SymbolMT,Italic",
  "SymbolMT,Bold",
  "SymbolMT,BoldItalic",
  "ZapfDingbats",
];

/// Mirrors `pdf_clean_font_name` in `MuPDF`'s `pdf-font.c`.
fn is_base14(name: &str) -> bool {
  let name: String = name.chars().filter(|c| *c != ' ').collect();
  BASE14.contains(&name.as_str())
}

/// Drops a subset tag such as `ABCDEF+`.
fn unsubset(name: &str) -> &str {
  match name.split_once('+') {
    Some((tag, rest))
      if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) =>
    {
      rest
    }
    _ => name,
  }
}

/// The family part of `Name-Style` or `Name,Style`.
fn family(name: &str) -> &str {
  name.split([',', '-']).next().unwrap_or(name)
}

fn from_bytes(name: &str, data: &[u8], index: u32) -> Option<Font> {
  Font::from_bytes_with_index(name, i32::try_from(index).ok()?, data).ok()
}

#[cfg(target_os = "macos")]
mod system {
  use super::{family, from_bytes};
  use font_kit::family_name::FamilyName;
  use font_kit::handle::Handle;
  use font_kit::properties::{Properties, Style, Weight};
  use font_kit::source::SystemSource;
  use mupdf::{CjkFontOrdering, Font, FontHints};

  pub fn named(name: &str, hints: FontHints) -> Option<Font> {
    let source = SystemSource::new();
    let handle = source.select_by_postscript_name(name).or_else(|_| {
      let mut properties = Properties::new();
      properties
        .weight(if hints.bold { Weight::BOLD } else { Weight::NORMAL })
        .style(if hints.italic { Style::Italic } else { Style::Normal });
      let names = [FamilyName::Title(family(name).to_owned())];
      source.select_best_match(&names, &properties)
    });
    load(name, &handle.ok()?)
  }

  /// Fonts that ship in `/System/Library/Fonts`.
  pub fn cjk(ordering: CjkFontOrdering, serif: bool) -> Option<Font> {
    use CjkFontOrdering as O;
    let name = match (ordering, serif) {
      (O::AdobeJapan, false) => "HiraginoSans-W3",
      (O::AdobeJapan, true) => "HiraMinProN-W3",
      (O::AdobeGb, false) => "STHeitiSC-Light",
      (O::AdobeGb, true) => "STSongti-SC-Regular",
      (O::AdobeCns, false) => "STHeitiTC-Light",
      (O::AdobeCns, true) => "STSongti-TC-Regular",
      (O::AdobeKorea, false) => "AppleSDGothicNeo-Regular",
      (O::AdobeKorea, true) => "AppleMyungjo",
    };
    let handle = SystemSource::new().select_by_postscript_name(name).ok()?;
    load(name, &handle)
  }

  fn load(name: &str, handle: &Handle) -> Option<Font> {
    match handle {
      Handle::Path { path, font_index } => {
        from_bytes(name, &std::fs::read(path).ok()?, *font_index)
      }
      Handle::Memory { bytes, font_index } => {
        from_bytes(name, bytes, *font_index)
      }
    }
  }
}

#[cfg(not(target_os = "macos"))]
mod system {
  use super::{family, from_bytes};
  use mupdf::{CjkFontOrdering, Font, FontHints};
  use std::process::{Command, Stdio};

  pub fn named(name: &str, hints: FontHints) -> Option<Font> {
    let mut pattern =
      format!("{}:postscriptname={}", escape(family(name)), escape(name));
    if hints.bold {
      pattern.push_str(":weight=bold");
    }
    if hints.italic {
      pattern.push_str(":slant=italic");
    }
    let found = fc_match(&pattern)?;
    let same = is_same(name, &found.family, &found.postscript);
    same.then(|| found.load(name)).flatten()
  }

  /// fc-match always answers, so unrelated fonts must fail.
  pub(super) fn is_same(name: &str, family: &str, postscript: &str) -> bool {
    let wanted = fold(name);
    let found = fold(family);
    !wanted.is_empty()
      && !found.is_empty()
      && (wanted == fold(postscript)
        || found == wanted
        || found == fold(super::family(name)))
  }

  pub fn cjk(ordering: CjkFontOrdering, serif: bool) -> Option<Font> {
    use CjkFontOrdering as O;
    let lang = match ordering {
      O::AdobeJapan => "ja",
      O::AdobeGb => "zh-cn",
      O::AdobeCns => "zh-tw",
      O::AdobeKorea => "ko",
    };
    let generic = if serif { "serif" } else { "sans-serif" };
    let found = fc_match(&format!("{generic}:lang={lang}"))?;
    let covers = found.langs.split('|').any(|l| l == lang);
    covers.then(|| found.load(&found.family)).flatten()
  }

  struct Match {
    file: String,
    index: u32,
    family: String,
    postscript: String,
    langs: String,
  }

  impl Match {
    fn load(&self, name: &str) -> Option<Font> {
      from_bytes(name, &std::fs::read(&self.file).ok()?, self.index)
    }
  }

  fn fc_match(pattern: &str) -> Option<Match> {
    const FORMAT: &str =
      "%{file}\n%{index}\n%{family[0]}\n%{postscriptname}\n%{lang}";
    let output = Command::new("fc-match")
      .args(["--format", FORMAT, pattern])
      .stdin(Stdio::null())
      .stderr(Stdio::null())
      .output()
      .ok()?;
    if !output.status.success() {
      return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let mut fields = text.split('\n').map(str::to_owned);
    Some(Match {
      file: fields.next().filter(|f| !f.is_empty())?,
      index: fields.next()?.parse().ok()?,
      family: fields.next()?,
      postscript: fields.next()?,
      langs: fields.next().unwrap_or_default(),
    })
  }

  /// Escapes fontconfig pattern syntax.
  pub(super) fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
      if matches!(c, '\\' | '-' | ':' | ',') {
        out.push('\\');
      }
      out.push(c);
    }
    out
  }

  /// Lowercase alphanumerics, so `MS Mincho` meets `MS-Mincho`.
  pub(super) fn fold(name: &str) -> String {
    name
      .chars()
      .filter(char::is_ascii_alphanumeric)
      .map(|c| c.to_ascii_lowercase())
      .collect()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn names() {
    let cases = [
      ("Helvetica", "Helvetica", true, "Helvetica"),
      ("ABCDEF+Arial-BoldMT", "Arial-BoldMT", true, "Arial"),
      ("Times New Roman", "Times New Roman", true, "Times New Roman"),
      ("Calibri,Bold", "Calibri,Bold", false, "Calibri"),
      ("abcdef+MS-Mincho", "abcdef+MS-Mincho", false, "abcdef+MS"),
      ("Arial-Black", "Arial-Black", false, "Arial"),
    ];
    for (raw, clean, base14, fam) in cases {
      assert_eq!(unsubset(raw), clean, "{raw}");
      assert_eq!(is_base14(clean), base14, "{raw}");
      assert_eq!(family(clean), fam, "{raw}");
    }
  }

  #[cfg(not(target_os = "macos"))]
  #[test]
  fn fontconfig_matches() {
    let cases = [
      ("Lato-Bold", "Lato", "Lato-Bold", true),
      ("MS-Mincho", "MS Mincho", "MS-Mincho", true),
      ("Calibri,Bold", "Calibri", "Calibri-Bold", true),
      ("Cambria", "DejaVu Serif", "DejaVuSerif", false),
      ("DejaVuSansMono-Bold", "DejaVu Sans", "DejaVuSans-Bold", false),
      ("\u{FF2D}\u{FF33}", "DejaVu Sans", "", false),
    ];
    for (name, family, postscript, same) in cases {
      assert_eq!(system::is_same(name, family, postscript), same, "{name}");
    }
  }

  #[cfg(not(target_os = "macos"))]
  #[test]
  fn fontconfig_syntax() {
    let cases = [
      ("MS-Mincho", r"MS\-Mincho", "msmincho"),
      ("A:B,C\\D", r"A\:B\,C\\D", "abcd"),
      ("MS Mincho", "MS Mincho", "msmincho"),
    ];
    for (name, escaped, folded) in cases {
      assert_eq!(system::escape(name), escaped);
      assert_eq!(system::fold(name), folded);
    }
  }

  #[cfg(target_os = "macos")]
  #[test]
  fn macos_cjk_fonts_ship() {
    use CjkFontOrdering as O;
    for ordering in [O::AdobeJapan, O::AdobeGb, O::AdobeCns, O::AdobeKorea] {
      for serif in [false, true] {
        assert!(system::cjk(ordering, serif).is_some(), "{ordering:?} {serif}");
      }
    }
  }
}
