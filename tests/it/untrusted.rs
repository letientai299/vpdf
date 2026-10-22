use crate::common;

use std::fmt::Write as _;
use std::path::Path;
use vpdf::Result;
use vpdf::app::Tabline;
use vpdf::solo::Solo;
use vpdf_model::document::Target;
use vpdf_model::text::PageId;
use vpdf_pdf::document::Document;
use vpdf_ui::keys::PanelKind;

/// OSC, DCS, C1 OSC, and C1 CSI.
const EVIL: &str = "\x1b]2;pwn\x07\x1bP0;1|\x1b\\\u{9d}\u{9b}\n";

/// Bytes 0x18-0x1F force `PDFDocEncoding`; UTF-16 keeps them.
fn text(value: &str) -> String {
  let mut hex = String::from("<FEFF");
  for unit in value.encode_utf16() {
    write!(hex, "{unit:04X}").unwrap();
  }
  hex + ">"
}

/// Every document string carries `EVIL`.
fn write_pdf(path: &Path, trailer: &str) -> Result<()> {
  let objects = [
    format!(
      "<< /Type /Catalog /Pages 3 0 R /Outlines 5 0 R \
       /PageLabels << /Nums [0 << /S /D /P {} >>] >> >>",
      text(EVIL)
    ),
    format!("<< /Title {} /Author {} >>", text(EVIL), text(EVIL)),
    "<< /Type /Pages /Kids [4 0 R] /Count 1 >>".to_owned(),
    "<< /Type /Page /Parent 3 0 R /MediaBox [0 0 200 200] \
     /Annots [7 0 R 8 0 R] >>"
      .to_owned(),
    "<< /Type /Outlines /First 6 0 R /Last 6 0 R /Count 1 >>".to_owned(),
    format!(
      "<< /Title {} /Parent 5 0 R /Dest [4 0 R /XYZ 0 200 0] >>",
      text(EVIL)
    ),
    format!(
      "<< /Type /Annot /Subtype /Link /Rect [10 10 100 30] \
       /A << /S /URI /URI {} >> >>",
      text(&format!("https://example.com/{EVIL}"))
    ),
    format!(
      "<< /Type /Annot /Subtype /Text /Rect [10 50 30 70] /Contents {} \
       /T {} >>",
      text(EVIL),
      text(EVIL)
    ),
  ];
  common::write_pdf(path, &objects, &format!("/Info 2 0 R {trailer}"))
}

fn assert_printable(field: &str, value: &str) {
  assert!(value.contains("]2;pwn"), "{field} kept its text: {value:?}");
  assert!(!value.chars().any(char::is_control), "{field}: {value:?}");
}

#[test]
fn document_strings_reach_the_screen_clean() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join(format!("{}.pdf", EVIL.trim_end()));
  write_pdf(&path, "")?;
  let mut doc = Document::open(&path, "")?;
  doc.load_outline()?;
  let Target::Uri(uri) = &doc.links(PageId(0))?[0].target else {
    panic!("URI link");
  };
  let notes = doc.annotations(PageId(0))?;
  let metadata = doc.metadata();
  let author = metadata.iter().find(|(key, _)| key == "Author").unwrap();
  for (field, value) in [
    ("page label", doc.label(PageId(0))),
    ("outline title", &doc.outline[0].title),
    ("document title", &doc.title),
    ("Info entry", &author.1),
    ("note", &notes[0].contents),
    ("note author", &notes[0].author),
    ("link", uri),
  ] {
    assert_printable(field, value);
  }
  assert_eq!(uri, "https://example.com/]2;pwnP0;1|\\");

  let screen = common::cells(160, 30);
  // Graphics add tmux DCS wrappers.
  let text_mode = true;
  let mut app = Solo::open(&path, "", text_mode, screen)?;
  common::outline_loaded(&mut app)?;
  assert_printable("file name", &app.name());
  assert_printable("window title", &app.title());
  app.tabline = Tabline { names: vec![app.name(), app.name()], active: 0 };
  for panel in [
    None,
    Some(PanelKind::Outline),
    Some(PanelKind::Info),
    Some(PanelKind::Annotations),
    Some(PanelKind::Links),
  ] {
    if let Some(kind) = panel {
      app.open_panel(kind)?;
      common::sidebar_loaded(&mut app, kind)?;
      let rows = &app.panel.as_ref().unwrap().view.rows();
      let shown = rows.iter().any(|row| row.label.contains("]2;pwn"));
      assert!(shown, "{kind:?} lists the text");
    }
    let mut frame = Vec::new();
    {
      let (app, graphics) = app.parts();
      app.draw_pane(graphics, &mut frame)?;
    }
    let frame = String::from_utf8_lossy(&frame);
    assert!(frame.contains("]2;pwn"), "{panel:?} shows the text");
    let raw = frame.contains("\x1b]")
      || frame.contains("\x1bP")
      || frame.contains('\x07')
      || frame.chars().any(|ch| ('\u{80}'..='\u{9f}').contains(&ch));
    assert!(!raw, "{panel:?}: {frame:?}");
  }
  Ok(())
}

/// `MuPDF` quotes `/Filter` names in errors.
#[test]
fn errors_reach_stderr_clean() -> Result<()> {
  let (_directory, path) = common::temp_pdf(|path| {
    write_pdf(path, "/Encrypt << /Filter /Evil#1B#5D2;pwn#07 >>")
  })?;
  let output = std::process::Command::new(env!("CARGO_BIN_EXE_vpdf"))
    .env(vpdf::inline::VAR, "text")
    .arg(&path)
    .output()?;
  let stderr = String::from_utf8_lossy(&output.stderr);
  assert!(!output.status.success() && stderr.contains("]2;pwn"), "{stderr:?}");
  assert!(!stderr.contains(['\x1b', '\x07']), "{stderr:?}");
  Ok(())
}

/// Tests never probe the terminal.
#[test]
fn info_says_the_terminal_was_not_probed() -> Result<()> {
  let (_directory, path) = common::temp_pdf(|path| write_pdf(path, ""))?;
  let screen = common::cells(80, 24);
  let app = Solo::open(&path, "", true, screen)?;
  let info = app.info();
  let reader = info.split_once("# Reader\n").map_or("", |(_, rest)| rest);
  assert!(reader.lines().any(|line| line == "Terminal: not probed"), "{info}");
  Ok(())
}

/// Unsorted keys, a first range after page 0, an unknown style, a
/// negative start, and entries that are not dictionaries.
#[test]
fn malformed_page_labels_fall_back() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("labels.pdf");
  let pages = 5;
  let kids: Vec<_> =
    (0..pages).map(|page| format!("{} 0 R", page + 3)).collect();
  let mut objects = vec![
    format!(
      "<< /Type /Catalog /Pages 2 0 R /PageLabels << /Nums [3 << /S /Q \
       /St -4 /P {} >> 1 (junk) 4 7 2 << /S /r /St 0 >>] >> >>",
      text("\u{1b}x")
    ),
    format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" ")),
  ];
  objects.extend((0..pages).map(|_| {
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>".to_owned()
  }));
  common::write_pdf(&path, &objects, "")?;
  let doc = Document::open(&path, "")?;
  for page in (0..pages).map(PageId) {
    let label = doc.label(page);
    assert!(!label.is_empty(), "{page:?}");
    assert!(!label.chars().any(char::is_control), "{page:?}: {label:?}");
  }
  assert_eq!(doc.page_number("junk"), None);
  assert_eq!(doc.page_number("2"), Some(PageId(1)));
  Ok(())
}
