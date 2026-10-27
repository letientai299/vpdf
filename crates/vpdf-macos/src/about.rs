use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadOnly};
use objc2_app_kit::{
  NSApplication, NSBackingStoreType, NSColor, NSFont, NSFontAttributeName,
  NSForegroundColorAttributeName, NSImageView, NSLinkAttributeName,
  NSMutableParagraphStyle, NSPanel, NSParagraphStyleAttributeName, NSTextField,
  NSTextView, NSWindowStyleMask,
};
use objc2_foundation::{
  MainThreadMarker, NSDictionary, NSMutableAttributedString, NSPoint, NSRange,
  NSRect, NSSize, NSString, ns_string,
};

const WIDTH: f64 = 620.0;
const INSET: f64 = 24.0;

pub struct About<'a> {
  pub version: &'a str,
  pub description: &'a str,
  pub repository: &'a str,
  pub build: &'a str,
}

pub(super) fn panel(
  about: &About<'_>,
  mtm: MainThreadMarker,
) -> Retained<NSPanel> {
  let header = header(about);
  let header = text_view(&header, mtm);
  let diagnostics = text_view(&attributed(about.build), mtm);
  let height =
    176.0 + header.frame().size.height + diagnostics.frame().size.height;
  let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
    NSPanel::alloc(mtm),
    rect(0.0, 0.0, WIDTH, height),
    NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
    NSBackingStoreType::Buffered,
    false,
  );
  // SAFETY: Callback owns the panel through close/reopen.
  unsafe { panel.setReleasedWhenClosed(false) };
  panel.setTitle(ns_string!("About Vpdf"));
  panel.setHidesOnDeactivate(false);
  panel.setFloatingPanel(false);
  let content = panel.contentView().expect("About content view");
  if let Some(image) =
    NSApplication::sharedApplication(mtm).applicationIconImage()
  {
    let icon = NSImageView::imageViewWithImage(&image, mtm);
    icon.setFrame(rect(INSET, height - 88.0, 64.0, 64.0));
    content.addSubview(&icon);
  }
  for (text, size, y) in
    [("Vpdf", 20.0, height - 48.0), (about.version, 12.0, height - 72.0)]
  {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(size)));
    label.setSelectable(true);
    label.setFrame(rect(104.0, y, WIDTH - 128.0, 26.0));
    content.addSubview(&label);
  }
  let top = height - 104.0 - header.frame().size.height;
  header.setFrameOrigin(NSPoint::new(INSET, top));
  content.addSubview(&header);
  let label = NSTextField::labelWithString(ns_string!("Diagnostics"), mtm);
  label.setFrame(rect(INSET, top - 36.0, WIDTH - 2.0 * INSET, 20.0));
  content.addSubview(&label);
  diagnostics.setFrameOrigin(NSPoint::new(INSET, INSET));
  content.addSubview(&diagnostics);
  panel
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
  NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

fn text_view(
  text: &NSMutableAttributedString,
  mtm: MainThreadMarker,
) -> Retained<NSTextView> {
  let width = WIDTH - INSET * 2.0;
  let view = NSTextView::initWithFrame(
    NSTextView::alloc(mtm),
    rect(0.0, 0.0, width, 32.0),
  );
  view.setEditable(false);
  view.setSelectable(true);
  view.setDrawsBackground(false);
  view.setTextContainerInset(NSSize::new(0.0, 0.0));
  // SAFETY: View retains its text system throughout layout.
  unsafe {
    let storage = view.textStorage().expect("About text storage");
    storage.setAttributedString(text);
    let container = view.textContainer().expect("About text container");
    container.setContainerSize(NSSize::new(width, f64::MAX));
    let layout = view.layoutManager().expect("About text layout");
    layout.ensureLayoutForTextContainer(&container);
    let height = layout.usedRectForTextContainer(&container).size.height;
    view.setFrameSize(NSSize::new(width, height.ceil() + 4.0));
  }
  view
}

fn header(about: &About<'_>) -> Retained<NSMutableAttributedString> {
  let text = format!("{}\n\nProject · Report an issue", about.description);
  let header = attributed(&text);
  let start = format!("{}\n\n", about.description).encode_utf16().count();
  let issues = format!("{}/issues", about.repository.trim_end_matches('/'));
  for (offset, label, url) in [
    (0, "Project", about.repository),
    ("Project · ".encode_utf16().count(), "Report an issue", &issues),
  ] {
    link(&header, NSRange::new(start + offset, label.len()), url);
  }
  header
}

fn attributed(text: &str) -> Retained<NSMutableAttributedString> {
  let font = NSFont::systemFontOfSize(12.0);
  let color = NSColor::labelColor();
  let style = NSMutableParagraphStyle::new();
  style.setLineSpacing(2.0);
  // SAFETY: Attribute values have their documented types.
  let result = unsafe {
    let attributes = NSDictionary::from_slices(
      &[
        NSFontAttributeName,
        NSForegroundColorAttributeName,
        NSParagraphStyleAttributeName,
      ],
      &[font.as_ref(), color.as_ref(), style.as_ref()],
    );
    NSMutableAttributedString::initWithString_attributes(
      NSMutableAttributedString::alloc(),
      &NSString::from_str(text),
      Some(&attributes),
    )
  };
  for (start, _) in text.match_indices("https://") {
    let url = text[start..].split_whitespace().next().unwrap();
    let range = NSRange::new(
      text[..start].encode_utf16().count(),
      url.encode_utf16().count(),
    );
    link(&result, range, url);
  }
  result
}

fn link(text: &NSMutableAttributedString, range: NSRange, url: &str) {
  // SAFETY: UTF-16 ranges cover their link labels.
  unsafe {
    text.addAttribute_value_range(
      NSLinkAttributeName,
      &NSString::from_str(url),
      range,
    );
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn separate_header_links() {
    let about = About {
      version: "abc1234 (2026-10-27) [bench-events]",
      description: "Vpdf 📖: a PDF reader.",
      repository: "https://example.com/vpdf",
      build: "Source revision: abc1234",
    };
    let header = header(&about);
    let text = header.string().to_string();
    assert!(text.starts_with(about.description));
    assert!(!text.contains(about.build));
    for (label, url) in [
      ("Project", about.repository.to_owned()),
      ("Report an issue", format!("{}/issues", about.repository)),
    ] {
      check_link(&header, &text, label, &url);
    }
  }

  #[test]
  fn diagnostic_links_handle_unicode() {
    let text = "📖 Source\nhttps://example.com/commit/abc\nHelp\n\
                https://example.com/blob/abc/scripts/help_assets.py";
    let result = attributed(text);
    assert_eq!(result.string().to_string(), text);
    for url in text.split_whitespace().filter(|s| s.starts_with("https://")) {
      check_link(&result, text, url, url);
    }
  }

  fn check_link(
    result: &NSMutableAttributedString,
    text: &str,
    label: &str,
    url: &str,
  ) {
    let index = text[..text.find(label).unwrap()].encode_utf16().count();
    // SAFETY: Index lies within the attributed string.
    let value = unsafe {
      result.attribute_atIndex_effectiveRange(
        NSLinkAttributeName,
        index,
        std::ptr::null_mut(),
      )
    }
    .unwrap();
    assert_eq!(value.downcast_ref::<NSString>().unwrap().to_string(), url);
  }
}
