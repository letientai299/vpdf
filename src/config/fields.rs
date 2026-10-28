//! Help metadata for each public setting.

/// Generated reference text for one setting.
#[derive(Debug)]
pub(crate) struct Field {
  pub name: &'static str,
  pub summary: &'static str,
  pub value: String,
  pub precedence: &'static str,
  pub reload: &'static str,
}

/// The shared entry for every `appearance.opacity` key.
const OPACITY: &str = "appearance.opacity.";

fn names<T: Copy>(values: &[T], name: impl Fn(T) -> &'static str) -> String {
  let names: Vec<_> =
    values.iter().map(|&value| format!("`{}`", name(value))).collect();
  names.join(", ")
}

fn fields() -> Vec<Field> {
  use vpdf_model::theme::Theme;
  use vpdf_ui::layout::{MAX_ZOOM, MIN_ZOOM};
  let all = "Reload applies it to every pane.";
  vec![
    Field {
      name: "appearance.theme",
      summary: "Interface color theme",
      value: format!(
        "{}, or a theme file path",
        names(Theme::ALL, Theme::name)
      ),
      precedence: "`:theme` replaces it until the next reload.",
      reload: all,
    },
    Field {
      name: "appearance.ascii_glyphs",
      summary: "ASCII panel symbols",
      value: "`true` or `false`".into(),
      precedence: "",
      reload: all,
    },
    Field {
      name: "appearance.ui_font_size",
      summary: "Native interface text size",
      value: format!(
        "integer from {} to {}",
        super::FONT_SIZES.start(),
        super::FONT_SIZES.end()
      ),
      precedence: "Native app only; the terminal uses its own font.",
      reload: all,
    },
    Field {
      name: "appearance.link_underline",
      summary: "Underline every link",
      value: "`true` or `false`".into(),
      precedence: "`:links` changes the current pane until the next reload.",
      reload: all,
    },
    Field {
      name: "appearance.link_focus",
      summary: "Emphasize the link under the cursor",
      value: "`true` or `false`".into(),
      precedence: "",
      reload: all,
    },
    Field {
      name: "appearance.link_width",
      summary: "Link decoration width in pixels",
      value: "integer from 1 to 4".into(),
      precedence: "",
      reload: all,
    },
    Field {
      name: OPACITY,
      summary: "Overlay opacity",
      value: "number from 0 to 1".into(),
      precedence: "An explicit value bypasses the page contrast cap.",
      reload: all,
    },
    Field {
      name: "reading.cursorline",
      summary: "Initial cursor line highlight",
      value: "`true` or `false`".into(),
      precedence: "`:cursorline` changes the current pane.",
      reload: "Reload applies it to newly opened documents.",
    },
    Field {
      name: "reading.zoom",
      summary: "Initial width-relative zoom",
      value: format!("number from {MIN_ZOOM} to {MAX_ZOOM}"),
      precedence: "Saved document zoom takes precedence.",
      reload: "Reload applies it to newly opened documents.",
    },
    Field {
      name: "reading.restore_session",
      summary: "Native session restore",
      value: format!(
        "one of {}",
        names(super::Restore::ALL, super::Restore::name)
      ),
      precedence: "Native app only; `system` follows macOS window reopening.",
      reload: "Read when the native app starts.",
    },
    Field {
      name: "ocr.language",
      summary: "OCR language codes",
      value: "Tesseract language codes joined by `+`".into(),
      precedence: "A `:ocr` language argument overrides it for that run.",
      reload: "Reload applies it to later OCR requests.",
    },
  ]
}

/// Metadata for a public setting name.
pub(crate) fn describe(name: &str) -> crate::Result<&'static Field> {
  static FIELDS: std::sync::OnceLock<Vec<Field>> = std::sync::OnceLock::new();
  let key = if name.starts_with(OPACITY) { OPACITY } else { name };
  FIELDS
    .get_or_init(fields)
    .iter()
    .find(|field| field.name == key)
    .ok_or_else(|| anyhow::anyhow!("Missing setting metadata: {name}"))
}

impl Field {
  /// The reference description after the default value.
  pub fn details(&self) -> String {
    let mut text = format!("Value: {}.", self.value);
    for sentence in [self.precedence, self.reload] {
      if !sentence.is_empty() {
        text.push(' ');
        text.push_str(sentence);
      }
    }
    text
  }
}
