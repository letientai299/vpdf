use super::*;
use crate::config::Settings;
use std::sync::{Arc, mpsc};
use std::time::Duration;
use vpdf_model::document::Target;
use vpdf_model::text::{PageId, Position};
use vpdf_pdf::document::{Document, Handle};
use vpdf_pdf::jobs::Wake;
use vpdf_ui::bindings::Context;
use vpdf_ui::topics::{Invocation, PaneContext, Reference, TopicId};

fn snapshot() -> Reference {
  Settings::default().help_reference(Invocation::Pane {
    context: Context::Normal,
    pane: PaneContext {
      id: 7,
      document: "Think Python.pdf".into(),
      document_generation: 12,
      position: Position { page: PageId(3), line: 4, glyph: 5 },
      zoom: 1.75,
      cursorline: false,
      text_mode: true,
      panel: None,
    },
  })
}

#[test]
fn reference_ignores_invoking_mode() -> crate::Result<()> {
  let reference = Settings::default().help_reference(Invocation::Home);
  let home = request(&mut Runtime::default(), &reference, "half_down")?;
  let pane = request(&mut Runtime::default(), &snapshot(), "half_down")?;
  assert_eq!(
    text(&home.document, "half_down")?,
    text(&pane.document, "half_down")?
  );
  let rendered = text(&home.document, "half_down")?;
  assert!(rendered.contains("<C-d>"));
  assert!(!rendered.contains("snapshot"));
  assert!(!rendered.contains("Unbound"));
  Ok(())
}

fn request(
  runtime: &mut Runtime,
  reference: &Reference,
  topic: &str,
) -> crate::Result<Ready> {
  let (send, receive) = mpsc::channel();
  let wake = Wake::new(move || {
    let _ = send.send(());
  });
  runtime.request(reference, TopicId::parse(topic)?, &wake);
  loop {
    if let Some(ready) = runtime.poll() {
      return ready;
    }
    receive.recv_timeout(Duration::from_secs(15))?;
  }
}

fn page(target: &Target) -> usize {
  match target {
    Target::Page(page) | Target::Point { page, .. } => page.0,
    Target::Uri(_) => panic!("External help destination"),
  }
}

fn text(
  document: &vpdf_pdf::manual::Document,
  topic: &str,
) -> crate::Result<String> {
  let native = PdfDocument::open(document.path())?;
  let coordinate = |target: &Target| match target {
    Target::Point { page, y, .. } => (page.0, *y),
    Target::Page(page) => (page.0, 0.0),
    Target::Uri(_) => unreachable!(),
  };
  let start = coordinate(&document.topics[topic]);
  let end = document
    .anchors
    .iter()
    .filter(|anchor| anchor.name.is_none())
    .map(|anchor| coordinate(&anchor.target))
    .filter(|&at| at > start)
    .min_by(|a, b| a.partial_cmp(b).unwrap())
    .unwrap_or((native.page_count()? as usize, 0.0));
  let handle = Handle::open(document.path(), "")?;
  let mut text = String::new();
  for page in start.0..=end.0.min(native.page_count()? as usize - 1) {
    for line in &handle.text(PageId(page))?.lines {
      for glyph in &line.glyphs {
        let at = (page, glyph.bounds.y0);
        if at >= start && at < end {
          text.push(glyph.ch);
        }
      }
      text.push('\n');
    }
  }
  Ok(text)
}

fn refresh(
  runtime: &mut Runtime,
  reference: &Reference,
) -> crate::Result<Ready> {
  let (send, receive) = mpsc::channel();
  let wake = Wake::new(move || {
    let _ = send.send(());
  });
  runtime.refresh(reference, &wake);
  loop {
    if let Some(ready) = runtime.poll() {
      return ready;
    }
    receive.recv_timeout(Duration::from_secs(15))?;
  }
}

#[test]
fn refresh_retains_view_anchor() -> crate::Result<()> {
  let mut runtime = Runtime::default();
  let mut reference = snapshot();
  let first = request(&mut runtime, &reference, "half_down")?;
  for topic in ["introduction", "first-session", "navigation", "sample"] {
    let target = &first.document.topics[topic];
    let page = PageId(page(target));
    let y = match target {
      Target::Point { y, .. } => *y,
      _ => 0.0,
    };
    for y in [0.0, y, y + 5.0] {
      let location = Location::at(&first.document, page, y).unwrap();
      assert_eq!(location.id().as_str(), topic);
    }
  }
  let target = &first.document.topics["navigation"];
  let y = match target {
    Target::Point { y, .. } => *y,
    _ => 0.0,
  };
  let location =
    Location::at(&first.document, PageId(page(target)), y).unwrap();
  assert_eq!(location.id().as_str(), "navigation");
  runtime.locate(location.clone());
  reference.configuration.generation += 1;
  let second = refresh(&mut runtime, &reference)?;
  assert_eq!(second.location, location);
  assert_eq!(second.target, second.document.topics["navigation"]);

  let target = &second.document.topics["config.reading.zoom"];
  let y = match target {
    Target::Point { y, .. } => *y,
    _ => 0.0,
  };
  let anchor = Location::at(&second.document, PageId(page(target)), y).unwrap();
  assert_eq!(
    anchor,
    Location::anchored(TopicId::parse("config")?, "reading.zoom")?
  );
  runtime.locate(anchor.clone());
  reference.configuration.settings.insert(
    0,
    vpdf_ui::reference::Setting {
      name: "testing.padding".into(),
      default: "".into(),
      effective: "long configuration value ".repeat(700).into(),
      source: vpdf_ui::reference::Source::default(),
    },
  );
  reference.configuration.generation += 1;
  let third = refresh(&mut runtime, &reference)?;
  assert_eq!(third.location, anchor);
  assert_eq!(third.target, third.document.topics["config.reading.zoom"]);
  assert_eq!(page(&third.target), page(target));
  assert!(third.fallback.is_none());
  Ok(())
}

#[test]
fn authored_section_refreshes() -> crate::Result<()> {
  let mut runtime = Runtime::default();
  let mut reference = snapshot();
  let first = request(&mut runtime, &reference, "navigation")?;
  let target = &first.document.topics["navigation.pages-and-printed-labels"];
  let Target::Point { page, y, .. } = target else { unreachable!() };
  let location = Location::at(&first.document, *page, *y).unwrap();
  assert_eq!(location.id().as_str(), "navigation");
  runtime.locate(location);
  reference.configuration.generation += 1;
  let second = refresh(&mut runtime, &reference)?;
  assert_eq!(second.target, *target);
  assert!(second.fallback.is_none());
  for spec in vpdf_ui::commands::COMMANDS {
    let topic = spec.topic();
    let content = text(&second.document, &topic)?;
    let plain = content.replace(['[', ']'], "");
    assert!(plain.contains(&format!(":{}", spec.name)), "{topic}");
    assert!(content.contains(spec.usage), "{topic}");
    for label in ["Available in ", "Not available in ", "Also accepts"] {
      assert!(!content.contains(label), "{topic}: {content}");
    }
  }
  Ok(())
}

#[test]
fn missing_topic_refreshes_with_fallback() -> crate::Result<()> {
  let mut runtime = Runtime::default();
  let first = request(&mut runtime, &snapshot(), "config")?;
  runtime.locate(Location::topic(TopicId::parse("retired-topic")?));
  let second = refresh(&mut runtime, &snapshot())?;
  assert_eq!(second.location.id().as_str(), "config");
  assert_eq!(second.fallback.unwrap().id().as_str(), "retired-topic");
  assert!(Arc::ptr_eq(&first.document, &second.document));
  Ok(())
}

#[test]
fn composed_document_reads() -> crate::Result<()> {
  let snapshot = snapshot();
  let registry = vpdf_ui::topics::Registry::new(
    vpdf_ui::topics::authored(),
    snapshot.bindings.clone(),
  )?;
  let ready = request(&mut Runtime::default(), &snapshot, "half_down")?;
  assert_eq!(ready.target, ready.document.topics["half_down"]);
  let mut doc = Document::open(ready.document.path(), "")?;
  let native = &doc.native;
  assert!(
    native
      .outlines()?
      .iter()
      .any(|row| row.title == "Built-in keybindings" && row.down.len() > 50)
  );
  for topic in registry.topics() {
    let id = topic.id.as_str();
    let target = native.resolve_link(&format!("#nameddest={id}"))?.unwrap();
    assert_eq!(
      target.loc.page_number as usize,
      page(&ready.document.topics[id])
    );
  }
  let mut forward = false;
  for index in 0..native.page_count()? {
    for link in native.load_page(index)?.links()? {
      assert!(!link.uri.starts_with("vpdf-help:"));
      if let Some(dest) = link.dest {
        assert!(dest.loc.page_number < native.page_count()? as u32);
        forward |= dest.loc.page_number as usize == page(&ready.target);
      }
    }
  }
  assert!(forward);
  assert!(!doc.links(PageId(page(&ready.target)))?.is_empty());
  let generated = doc.text(PageId(page(&ready.target)))?;
  let copied = generated
    .in_areas(
      PageId(page(&ready.target)),
      &[doc.pages[page(&ready.target)].bounds],
    )
    .1;
  assert!(copied.contains("<C-d>"));
  let index =
    vpdf_search::search::Index::new(PageId(page(&ready.target)), &generated);
  assert!(
    !index
      .matches(&vpdf_search::search::compile("<C-d>", false)?, 100)?
      .is_empty()
  );
  assert!(generated.lines.iter().any(|line| line.text().contains("<C-d>")));
  let config = text(&ready.document, "config")?;
  assert!(config.contains("Default: 1.0"));
  assert!(!config.contains("Snapshot"));
  let pixels = doc.native.load_page(page(&ready.target) as i32)?.to_pixmap(
    &mupdf::Matrix::new_scale(0.25, 0.25),
    &mupdf::Colorspace::device_rgb(),
    false,
    true,
  )?;
  assert!(pixels.samples().iter().any(|&v| v < 200));
  let other = Arc::clone(&ready.document);
  std::thread::spawn(move || {
    Handle::open(other.path(), "").unwrap().text(PageId(0)).unwrap()
  })
  .join()
  .unwrap();
  Ok(())
}

#[test]
fn refresh_keeps_builtin_content() -> crate::Result<()> {
  let mut runtime = Runtime::default();
  let mut reference = snapshot();
  let first = request(&mut runtime, &reference, "config")?;
  let same = request(&mut runtime, &reference, "half_down")?;
  assert!(Arc::ptr_eq(&first.document, &same.document));
  reference.configuration.generation += 1;
  reference.bindings.effective.retain(|row| row.target.topic() != "half_down");
  let second = request(&mut runtime, &reference, "config")?;
  assert!(Arc::ptr_eq(&first.document, &second.document));
  assert!(text(&second.document, "half_down")?.contains("<C-d>"));
  assert!(first.document.path().exists());
  let Invocation::Pane { pane, .. } = &mut reference.invocation else {
    unreachable!()
  };
  pane.id = 8;
  pane.zoom = 2.5;
  let third = request(&mut runtime, &reference, "config")?;
  assert!(Arc::ptr_eq(&first.document, &third.document));
  assert_eq!(
    text(&third.document, "config")?,
    text(&first.document, "config")?
  );
  assert_eq!(third.target, third.document.topics["config"]);
  let old = first.document.path().to_owned();
  drop(first);
  assert!(old.exists());
  drop(same);
  drop(second);
  assert!(old.exists());
  let current = third.document.path().to_owned();
  drop(third);
  drop(runtime);
  assert!(!current.exists());
  Ok(())
}

#[test]
fn reload_keeps_builtin_defaults() -> crate::Result<()> {
  let file = tempfile::NamedTempFile::new()?;
  std::fs::write(
    file.path(),
    concat!(
      "[reading]\nzoom = 2.0\n[keybindings.normal]\n",
      "h = { action = 'scroll', amount = 0.5 }\n"
    ),
  )?;
  let loaded = crate::config::Options {
    config: Some(file.path().into()),
    ..Default::default()
  }
  .load();
  assert!(loaded.warning.is_none(), "{:?}", loaded.warning);
  let reference = loaded.settings.help_reference(Invocation::Home);
  std::fs::write(file.path(), "[reading]\nzoom = 3.0\n")?;
  let ready = request(&mut Runtime::default(), &reference, "config")?;
  let config = text(&ready.document, "config")?;
  assert!(config.contains("Default: 1.0"));
  assert!(!config.contains("Effective:"));
  assert!(!config.contains(&file.path().display().to_string()));
  let action = text(&ready.document, "half_down")?;
  assert!(action.contains("<C-d>"), "{action}");
  Ok(())
}

#[test]
fn sample_note_refreshes() -> crate::Result<()> {
  let mut runtime = Runtime::default();
  let mut reference = snapshot();
  for theme in ["github-light", "nord", "github-light"] {
    reference
      .configuration
      .settings
      .iter_mut()
      .find(|setting| setting.name == "appearance.theme")
      .unwrap()
      .effective = theme.into();
    let ready = request(&mut runtime, &reference, "sample")?;
    let native = PdfDocument::open(ready.document.path())?;
    let notes: Vec<_> = native
      .load_pdf_page(page(&ready.target) as i32)?
      .annotations()
      .map(|note| note.contents().map(|text| text.map(str::to_owned)))
      .collect::<Result<_, _>>()?;
    assert_eq!(
      notes,
      [Some("Notes explain passages without changing your source PDF.".into())]
    );
  }
  Ok(())
}

#[test]
fn custom_commands_are_not_documented() -> crate::Result<()> {
  use vpdf_ui::bindings::Target;
  let file = tempfile::NamedTempFile::new()?;
  std::fs::write(
    file.path(),
    "[keybindings.normal]\nQ = { command = 'help sample' }\n",
  )?;
  let loaded = crate::config::Options {
    config: Some(file.path().into()),
    ..Default::default()
  }
  .load();
  assert!(loaded.warning.is_none());
  let reference = loaded.settings.help_reference(Invocation::Home);
  assert_eq!(
    reference.bindings,
    vpdf_ui::bindings::Bindings::default().snapshot()
  );
  let ready = request(&mut Runtime::default(), &reference, "half_down")?;
  assert!(
    !ready
      .document
      .topics
      .contains_key(&Target::Command("help sample".into()).topic())
  );
  assert!(
    !ready.document.topics.keys().any(|name| name.ends_with(".dispatch"))
  );
  Ok(())
}

#[test]
fn font_notices_include_license() -> crate::Result<()> {
  let ready = request(&mut Runtime::default(), &snapshot(), "config")?;
  for native in [
    PdfDocument::from_copied_bytes(PDF)?,
    PdfDocument::open(ready.document.path())?,
  ] {
    let outlines = native.outlines()?;
    let notices = outlines.last().unwrap();
    assert_eq!(notices.title, "Font notices");
    let start = notices.dest.unwrap().loc.page_number as i32;
    let first = native
      .load_page(start)?
      .to_text_page(TextPageFlags::empty())?
      .to_text()?;
    for needle in ["Font notices", "SIL OPEN FONT LICENSE"] {
      assert!(first.contains(needle), "Missing {needle}: {first}");
    }
    for index in start..native.page_count()? {
      let text = native
        .load_page(index)?
        .to_text_page(TextPageFlags::empty())?
        .to_text()?;
      assert!(!text.trim().is_empty(), "Empty font notice page: {index}");
    }
  }
  Ok(())
}

#[test]
fn theme_colors_and_monospace() -> crate::Result<()> {
  use vpdf_model::theme::Theme;
  let mut previous = None;
  for theme in [Theme::GithubLight, Theme::Nord] {
    let mut reference = snapshot();
    reference
      .configuration
      .settings
      .iter_mut()
      .find(|setting| setting.name == "appearance.theme")
      .unwrap()
      .effective = theme.name().into();
    let ready = request(&mut Runtime::default(), &reference, "half_down")?;
    let native = PdfDocument::open(ready.document.path())?;
    for index in [0, page(&ready.target) as i32] {
      let pixels = native.load_page(index)?.to_pixmap(
        &mupdf::Matrix::new_scale(0.25, 0.25),
        &mupdf::Colorspace::device_rgb(),
        false,
        true,
      )?;
      for (&actual, expected) in
        pixels.samples()[..3].iter().zip(theme.palette().background)
      {
        assert!(actual.abs_diff(expected) <= 1, "{actual} != {expected}");
      }
      let fonts = native
        .find_page(index)?
        .get_dict("Resources")?
        .unwrap()
        .get_dict("Font")?
        .unwrap();
      for font in fonts.dict_iter()? {
        let (_, font) = font?;
        let name = font.get_dict("BaseFont")?.unwrap().as_name()?;
        let name = String::from_utf8(name)?;
        assert!(name.replace(' ', "").contains("DejaVuSansMono"), "{name}");
      }
    }
    let rendered = text(&ready.document, "half_down")?;
    if let Some(previous) = &previous {
      assert_eq!(previous, &rendered);
    }
    previous = Some(rendered);
    let outlines = native.outlines()?;
    let appendix = outlines.last().unwrap();
    assert_eq!(appendix.title, "Font notices");
    assert!(
      appendix.dest.unwrap().loc.page_number > page(&ready.target) as u32
    );
    let authored_pages = PdfDocument::from_copied_bytes(PDF)?.page_count()?;
    assert!(
      native.page_count()? - authored_pages < 350,
      "Reference grouping exceeded budget: {} pages",
      native.page_count()?
    );
  }
  Ok(())
}

#[test]
fn private_values_are_omitted() -> crate::Result<()> {
  let mut reference = snapshot();
  let Invocation::Pane { pane, .. } = &mut reference.invocation else {
    unreachable!()
  };
  pane.document = "private-document-name.pdf".into();
  let setting = &mut reference.configuration.settings[0];
  setting.effective = "private-setting-value".into();
  setting.source =
    vpdf_ui::reference::Source::File("private-source.toml".into());
  reference.configuration.warnings.push("private-warning".into());
  let ready = request(&mut Runtime::default(), &reference, "config")?;
  let native = PdfDocument::open(ready.document.path())?;
  let mut content = String::new();
  for page in 0..native.page_count()? {
    content.push_str(
      &native
        .load_page(page)?
        .to_text_page(TextPageFlags::empty())?
        .to_text()?,
    );
  }
  for secret in [
    "private-document-name",
    "private-setting-value",
    "private-source",
    "private-warning",
  ] {
    assert!(!content.contains(secret), "{secret}");
  }
  let outlines = native.outlines()?;
  assert!(!outlines.iter().any(|row| matches!(
    row.title.as_str(),
    "Effective reference" | "Dispatch appendix"
  )));
  Ok(())
}

#[test]
fn commands_have_own_section() -> crate::Result<()> {
  let ready = request(&mut Runtime::default(), &snapshot(), "ex.edit")?;
  let native = PdfDocument::open(ready.document.path())?;
  let outlines = native.outlines()?;
  let effective =
    outlines.iter().find(|row| row.title == "Built-in keybindings").unwrap();
  let commands =
    outlines.iter().find(|row| row.title == "Built-in Ex commands").unwrap();
  assert!(effective.down.iter().all(|row| !row.title.starts_with(':')));
  assert_eq!(commands.down.len(), vpdf_ui::commands::COMMANDS.len());
  for (entry, spec) in commands.down.iter().zip(vpdf_ui::commands::COMMANDS) {
    assert_eq!(entry.title, format!(":{}", spec.name));
    let target = &ready.document.topics[&spec.topic()];
    assert_eq!(entry.dest.unwrap().loc.page_number as usize, page(target));
  }
  let first = commands.dest.unwrap().loc.page_number;
  assert!(
    effective
      .down
      .iter()
      .all(|entry| entry.dest.unwrap().loc.page_number < first)
  );
  let heading = native
    .load_page(first as i32)?
    .to_text_page(TextPageFlags::empty())?
    .to_text()?;
  assert!(heading.contains("Built-in Ex commands"));
  Ok(())
}

#[test]
fn references_lead_with_syntax() -> crate::Result<()> {
  let ready = request(&mut Runtime::default(), &snapshot(), "ex.edit")?;
  let command = text(&ready.document, "ex.edit")?;
  assert!(command.trim_start().starts_with(":e[dit] <file>"), "{command}");
  assert!(command.contains("Open a PDF in this tab."));
  for label in ["== :edit ==", "Syntax:", "Arguments:", "Aliases:", "Read:"] {
    assert!(!command.contains(label), "{command}");
  }
  let previous = text(&ready.document, "ex.tabprevious")?;
  assert!(
    previous.trim_start().starts_with(":tabp[revious]  :tabN"),
    "{previous}"
  );
  assert!(!previous.contains("Also accepts"));
  let keys = text(&ready.document, "half_down")?;
  assert!(keys.contains("<C-d>"), "{keys}");
  assert!(!keys.contains("built-in default"), "{keys}");
  assert!(!keys.contains("Snapshot mode:"), "{keys}");
  assert!(!ready.document.topics.contains_key("half_down.dispatch"));
  Ok(())
}

#[test]
fn command_colors_match_semantics() -> crate::Result<()> {
  use vpdf_model::theme::Theme;
  for theme in [Theme::GithubLight, Theme::Nord] {
    let mut reference = snapshot();
    reference
      .configuration
      .settings
      .iter_mut()
      .find(|setting| setting.name == "appearance.theme")
      .unwrap()
      .effective = theme.name().into();
    let guide = vpdf_ui::topics::authored()
      .into_iter()
      .find(|topic| topic.id.as_str() == "zoom")
      .unwrap()
      .title;
    let guide = format!("|{guide}|");
    let ready = request(&mut Runtime::default(), &reference, "ex.cursorline")?;
    let native = PdfDocument::open(ready.document.path())?;
    let palette = theme.palette();
    for (topic, needle, expected) in [
      ("ex.tabprevious", ":tabp[revious]", palette.magenta),
      ("ex.tabprevious", ":tabN", palette.magenta),
      ("ex.cursorline", "[on|off]", palette.yellow),
      ("ex.cursorline", "default on", palette.yellow),
      ("ex.cursorline", guide.as_str(), palette.cyan),
    ] {
      let glyphs =
        colored_glyphs(&native, page(&ready.document.topics[topic]))?;
      let text: String = glyphs.iter().map(|(ch, _)| ch).collect();
      let at = text.find(needle).unwrap_or_else(|| panic!("{needle}: {text}"));
      let start = text[..at].chars().count();
      let range = if needle == "default on" {
        start + "default ".len()..start + needle.len()
      } else {
        start..start + needle.chars().count()
      };
      for &(_, argb) in &glyphs[range] {
        let actual = [(argb >> 16) as u8, (argb >> 8) as u8, argb as u8];
        for (actual, expected) in actual.into_iter().zip(expected) {
          assert!(actual.abs_diff(expected) <= 1, "{topic}: {needle}");
        }
      }
    }
  }
  Ok(())
}

fn colored_glyphs(
  native: &PdfDocument,
  page: usize,
) -> crate::Result<Vec<(char, u32)>> {
  let raw =
    native.load_page(page as i32)?.to_text_page(TextPageFlags::empty())?;
  let mut glyphs = Vec::new();
  for block in raw.blocks() {
    for line in block.lines() {
      glyphs.extend(
        line
          .chars()
          .filter_map(|glyph| glyph.char().map(|ch| (ch, glyph.argb()))),
      );
    }
  }
  Ok(glyphs)
}

#[test]
fn cancellation_reference_is_concise() -> crate::Result<()> {
  let topic =
    vpdf_ui::bindings::Target::Reader(vpdf_ui::keys::Action::Cancel).topic();
  let ready = request(&mut Runtime::default(), &snapshot(), &topic)?;
  let rendered = text(&ready.document, &topic)?;
  assert_eq!(rendered.matches("<Esc>").count(), 1, "{rendered}");
  assert!(!rendered.contains("<A-"), "{rendered}");
  assert!(!rendered.contains("<C-"), "{rendered}");
  assert_eq!(rendered.matches("cancel in").count(), 1, "{rendered}");
  for mode in ["normal", "cursor", "visual", "operator", "object"] {
    assert!(rendered.contains(mode), "{rendered}");
  }
  Ok(())
}

#[test]
fn mark_templates_share_destinations() -> crate::Result<()> {
  use vpdf_ui::bindings::Target as BindingTarget;
  use vpdf_ui::keys::Action;
  let ready = request(&mut Runtime::default(), &snapshot(), "marks")?;
  for (action, signature) in [
    (Action::Mark('a'), "m{mark}"),
    (Action::GoMark('a', false), "'{mark}"),
    (Action::GoMark('a', true), "`{mark}"),
  ] {
    let topic = BindingTarget::Reader(action).topic();
    let rendered = text(&ready.document, &topic)?;
    assert!(rendered.contains(signature), "{rendered}");
    let other = match action {
      Action::Mark(_) => Action::Mark('Z'),
      Action::GoMark(_, exact) => Action::GoMark('Z', exact),
      _ => unreachable!(),
    };
    assert_eq!(
      ready.document.topics[&topic],
      ready.document.topics[&BindingTarget::Reader(other).topic()]
    );
    let ex = &ready.document.topics["ex.config"];
    assert!(page(&ready.document.topics[&topic]) < page(ex));
  }
  Ok(())
}

#[test]
fn hint_labels_share_destination() -> crate::Result<()> {
  use vpdf_ui::bindings::{HintAction, Target as BindingTarget};
  let ready = request(&mut Runtime::default(), &snapshot(), "links")?;
  let first = BindingTarget::Hint(HintAction::Type { character: 'a' }).topic();
  let rendered = text(&ready.document, &first)?;
  assert!(rendered.contains("{label}"), "{rendered}");
  assert!(!rendered.contains("hint type; character:"), "{rendered}");
  assert!(rendered.contains("|Links and notes|"), "{rendered}");
  for character in 'a'..='z' {
    let topic = BindingTarget::Hint(HintAction::Type { character }).topic();
    assert_eq!(ready.document.topics[&first], ready.document.topics[&topic]);
  }
  Ok(())
}

#[test]
fn theme_change_replaces_cached_pdf() -> crate::Result<()> {
  let mut runtime = Runtime::default();
  let mut reference = snapshot();
  let setting = reference
    .configuration
    .settings
    .iter_mut()
    .find(|setting| setting.name == "appearance.theme")
    .unwrap();
  setting.effective = "github-light".into();
  let first = request(&mut runtime, &reference, "config")?;
  reference
    .configuration
    .settings
    .iter_mut()
    .find(|setting| setting.name == "appearance.theme")
    .unwrap()
    .effective = "nord".into();
  let second = request(&mut runtime, &reference, "config")?;
  assert!(!Arc::ptr_eq(&first.document, &second.document));
  assert_eq!(
    text(&first.document, "config")?,
    text(&second.document, "config")?
  );
  Ok(())
}
