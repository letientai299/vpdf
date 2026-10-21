use crate::find::Search;
use crate::layout::{Layout, Screen};
use crate::text::Bounds;
use std::path::Path;

pub struct DocumentInfo<'a> {
  pub metadata: &'a [(String, String)],
  pub path: &'a Path,
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
  pub fn text(&self) -> String {
    let doc = &self.document;
    let mut lines = metadata_lines(doc.metadata);
    lines.push(String::new());
    lines.extend(document_lines(
      format!("Path: {}", doc.path.display()),
      doc.bytes,
      doc.pages,
    ));
    let (width, height) = (doc.bounds.width(), doc.bounds.height());
    let outline = if doc.inferred { " (inferred)" } else { "" };
    lines.extend([
      format!(
        "Page {}: {width:.0} x {height:.0} pt, {:.0} x {:.0} mm",
        doc.label,
        width / 72.0 * 25.4,
        height / 72.0 * 25.4
      ),
      format!("Outline: {} entries{outline}", doc.headings),
      format!("Marks: {}", doc.marks),
      String::new(),
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
      format!("Mouse: {}", if self.mouse { "on" } else { "off (:mouse on)" }),
      self.frontend.into(),
    ]);
    lines.join("\n")
  }
}

pub fn metadata_lines(metadata: &[(String, String)]) -> Vec<String> {
  let fields = metadata.iter().map(|(k, v)| format!("{k}: {v}"));
  std::iter::once("# Metadata".to_owned()).chain(fields).collect()
}

/// `:info`'s file facts, shared with recent document cards; `place`
/// names where the file lives.
pub fn document_lines(place: String, bytes: u64, pages: usize) -> Vec<String> {
  vec![
    "# Document".into(),
    place,
    format!("Size: {:.1} MiB", bytes as f64 / 1_048_576.0),
    format!("Pages: {pages}"),
  ]
}

pub fn help(installed: impl Fn(&str) -> bool) -> Vec<String> {
  let reference = crate::keys::reference();
  let keys = reference.lines().map(str::to_owned);
  let commands = crate::commands::commands(installed).map(|spec| {
    let usage = if spec.usage.is_empty() { "" } else { " " };
    format!(":{}{usage}{}: {}", spec.name, spec.usage, spec.help)
  });
  keys.chain([String::new(), "# Commands".into()]).chain(commands).collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn frontend_facts_preserve_order() {
    let screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
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
