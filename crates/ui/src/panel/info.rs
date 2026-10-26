use crate::find::Search;
use crate::layout::{Layout, Screen};
use crate::text::Bounds;
use std::path::Path;

pub struct DocumentInfo<'a> {
  pub metadata: &'a [(String, String)],
  pub path: &'a Path,
  /// Shown as `~` in the folder.
  pub home: Option<&'a Path>,
  pub bytes: u64,
  pub pages: usize,
  pub label: &'a str,
  pub bounds: Bounds,
  pub headings: usize,
  pub inferred: bool,
  pub marks: usize,
  pub cache_bytes: usize,
}

pub struct Info<'a> {
  pub document: DocumentInfo<'a>,
  pub layout: &'a Layout,
  pub search: &'a Search,
  pub images: &'a [String],
  pub render_ms: f64,
  pub screen: Screen,
  pub text: bool,
  pub mouse: bool,
  pub frontend: &'a str,
}

impl Info<'_> {
  /// A header block, then `# Section` groups of `key: value` lines.
  pub fn text(&self) -> String {
    let doc = &self.document;
    let name = doc.path.file_name().unwrap_or_default().to_string_lossy();
    let folder = doc.path.parent().unwrap_or(Path::new(""));
    let folder = super::content::tilde(folder, doc.home);
    let size = doc.bytes as f64 / 1_048_576.0;
    // The format and encryption join the facts, not the metadata.
    let (facts, metadata): (Vec<_>, Vec<_>) = doc
      .metadata
      .iter()
      .cloned()
      .partition(|(key, _)| matches!(key.as_str(), "Format" | "Encryption"));
    let mut strip =
      vec![format!("{} pages", doc.pages), format!("{size:.1} MiB")];
    strip.extend(facts.into_iter().map(|(key, value)| match key.as_str() {
      "Encryption" if value == "None" => "no encryption".to_owned(),
      "Encryption" => format!("{value} encryption"),
      _ => value,
    }));
    let mut lines = vec![
      line(&name),
      line(&folder.display().to_string()),
      line(&strip.join(" · ")),
    ];
    let (width, height) = (doc.bounds.width(), doc.bounds.height());
    let outline = if doc.inferred { " (inferred)" } else { "" };
    lines.extend([
      "# Document".into(),
      format!(
        "Page {}: {width:.0} × {height:.0} pt · {:.0} × {:.0} mm",
        doc.label,
        width / 72.0 * 25.4,
        height / 72.0 * 25.4
      ),
      format!("Outline: {} entries{outline}", doc.headings),
      format!("Marks: {}", doc.marks),
    ]);
    lines.extend(metadata_lines(&metadata));
    lines.extend([
      "# Reader".into(),
      format!("Zoom: {:.0}%", self.layout.zoom() * 100.0),
      format!("Rotation: {}°", self.layout.rotation()),
      format!(
        "Margins: {}",
        if self.layout.crop().is_none() { "shown" } else { "cropped" }
      ),
      format!(
        "Search: {} hits for {:?}",
        self.search.hits().len(),
        self.search.query().text
      ),
      format!("# {RENDER}"),
      format!("Text cache: {} KiB", doc.cache_bytes / 1024),
    ]);
    lines.extend(self.images.iter().cloned());
    lines.extend([
      format!("Last tile: {:.3} ms", self.render_ms),
      format!(
        "Cell: {}x{} px",
        self.screen.cell_width, self.screen.cell_height
      ),
      format!("Screen: {}x{} cells", self.screen.columns, self.screen.rows),
      format!("Presentation: {}", if self.text { "text" } else { "images" }),
      format!("# {TERMINAL}"),
      format!("Mouse: {}", if self.mouse { "on" } else { "off (:mouse on)" }),
      self.frontend.into(),
    ]);
    lines.join("\n")
  }
}

/// Sections that start folded: Info's diagnostics, Version's build.
pub const RENDER: &str = "Render";
pub const TERMINAL: &str = "Terminal";
pub const BUILD: &str = "Build";

/// A one-line value; document text cannot start a section, by a
/// newline or a leading `# `.
fn line(text: &str) -> String {
  let text = text.replace(['\n', '\r'], " ");
  match text.strip_prefix("# ") {
    Some(rest) => format!("#\u{a0}{rest}"),
    None => text,
  }
}

/// Metadata under its heading; nothing without fields.
pub fn metadata_lines(metadata: &[(String, String)]) -> Vec<String> {
  if metadata.is_empty() {
    return Vec::new();
  }
  let fields = metadata.iter().map(|(k, v)| line(&format!("{k}: {v}")));
  std::iter::once("# Metadata".to_owned()).chain(fields).collect()
}

/// A recent document card's file facts; `place` names where the file
/// lives.
pub fn document_lines(place: String, bytes: u64, pages: usize) -> Vec<String> {
  vec![
    "# Document".into(),
    place,
    format!("Size: {:.1} MiB", bytes as f64 / 1_048_576.0),
    format!("Pages: {pages}"),
  ]
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn file_names_stay_in_the_header() {
    let metadata = [("Title".to_owned(), "x\n# y".to_owned())];
    let rows = crate::panel::Row::sections(
      &[line("# x.pdf"), line("/a\n# b")]
        .into_iter()
        .chain(metadata_lines(&metadata))
        .collect::<Vec<_>>()
        .join("\n"),
    );
    let headings: Vec<_> = rows
      .iter()
      .filter(|row| row.style == crate::panel::Style::Heading)
      .map(|row| row.label.as_str())
      .collect();
    assert_eq!(headings, ["Metadata"]);
  }

  #[test]
  fn frontend_facts_preserve_order() {
    let screen = crate::testing::screen::cells(80, 24);
    let bounds = Bounds { x0: 0.0, y0: 0.0, x1: 612.0, y1: 792.0 };
    let layout = Layout::new([bounds], screen);
    let search = Search::default();
    let images =
      ["Images: 7 KiB resident".into(), "Transferred: 11 KiB".into()];
    for (images, frontend) in [
      (images.as_slice(), "Terminal: not probed"),
      (images.as_slice(), "Profile: Kitty\nGraphics query: OK"),
      (&[][..], "Native: ready"),
    ] {
      let text = Info {
        document: DocumentInfo {
          metadata: &[],
          path: Path::new("paper.pdf"),
          home: None,
          bytes: 1024,
          pages: 1,
          label: "1",
          bounds,
          headings: 0,
          inferred: false,
          marks: 0,
          cache_bytes: 2048,
        },
        layout: &layout,
        search: &search,
        images,
        render_ms: 1.25,
        screen,
        text: false,
        mouse: true,
        frontend,
      }
      .text();
      let expected = std::iter::once("Text cache: 2 KiB")
        .chain(images.iter().map(String::as_str))
        .chain(["Last tile: 1.250 ms"])
        .collect::<Vec<_>>()
        .join("\n");
      assert!(text.contains(&expected), "{text}");
      assert!(text.ends_with(frontend), "{text}");
      assert_eq!(text.contains("Transferred:"), !images.is_empty());
      assert_eq!(text.contains("Images:"), !images.is_empty());
    }
  }
}
