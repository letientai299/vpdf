#[path = "../../benches/first_use/isolate.rs"]
mod isolate;
#[path = "../../benches/first_use/sample.rs"]
mod sample;

use sample::{Interaction, Preparation, Request};
use std::collections::HashSet;
use std::process::Command;
use std::time::Duration;
use vpdf::Result;

fn linked_pdf(path: &std::path::Path) -> Result<()> {
  let stream = "BT /F1 18 Tf 20 100 Td (Open destination text) Tj ET";
  let content =
    format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len());
  super::common::write_pdf(
    path,
    &[
      "<< /Type /Catalog /Pages 2 0 R >>".into(),
      "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".into(),
      "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] \
     /Contents 4 0 R /Resources << /Font << /F1 7 0 R >> >> \
     /Annots [8 0 R] >>"
        .into(),
      content.clone(),
      "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] \
     /Contents 6 0 R /Resources << /Font << /F1 7 0 R >> >> >>"
        .into(),
      content,
      "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
      "<< /Type /Annot /Subtype /Link /Rect [18 94 220 118] \
     /Dest [5 0 R /XYZ 0 180 0] >>"
        .into(),
    ],
    "",
  )
}

#[test]
fn page_plan_covers_stride_multiples() {
  assert_eq!(sample::plan(0, 31), [] as [usize; 0]);
  for pages in [1, 2, 17, 34, 323] {
    let picks = sample::plan(pages, pages + 1);
    assert_eq!(picks.len(), pages + 1);
    assert_eq!(picks[0], picks[pages]);
    assert_eq!(
      picks[..pages].iter().copied().collect::<HashSet<_>>().len(),
      pages
    );
    assert!(picks.iter().all(|page| *page < pages));
  }
}

#[test]
fn preparation_preserves_real_actions() -> Result<()> {
  let file = tempfile::NamedTempFile::new()?;
  linked_pdf(file.path())?;
  for prepared in [Preparation::Cold, Preparation::List, Preparation::Text] {
    for name in [
      Interaction::Selection,
      Interaction::Cursor,
      Interaction::Hover,
      Interaction::Figures,
      Interaction::Preview,
    ] {
      let request = Request {
        path: file.path().into(),
        name,
        prepared,
        page: 0,
        rendering: false,
      };
      let value = sample::sample(&request)?;
      assert!(value.milliseconds.is_some_and(f64::is_finite), "{name:?}");
      assert_eq!(value.ready_ms, None);
      if matches!(prepared, Preparation::Text) {
        assert!(value.text_cache_bytes > 0);
      } else {
        assert_eq!(value.text_cache_bytes, 0);
      }
      if name == Interaction::Preview {
        assert_eq!(value.destination, Some(1));
      }
    }
  }
  Ok(())
}

#[test]
fn active_input_keeps_text_cold() -> Result<()> {
  let file = tempfile::NamedTempFile::new()?;
  linked_pdf(file.path())?;
  let value = sample::sample(&Request {
    path: file.path().into(),
    name: Interaction::Selection,
    prepared: Preparation::List,
    page: 0,
    rendering: true,
  })?;
  assert_eq!(value.text_cache_bytes, 0);
  assert!(value.ready_ms.unwrap() >= value.milliseconds.unwrap());
  Ok(())
}

#[test]
fn unavailable_scene_is_skipped() -> Result<()> {
  for (text, name) in [("", Interaction::Selection), ("X", Interaction::Cursor)]
  {
    let file = tempfile::NamedTempFile::new()?;
    super::common::text_pdf(file.path(), &[text])?;
    let value = sample::sample(&Request {
      path: file.path().into(),
      name,
      prepared: Preparation::Text,
      page: 0,
      rendering: false,
    })?;
    assert_eq!(value.milliseconds, None);
    assert_eq!(value.preparation_ms, None);
    assert_eq!(value.text_cache_bytes, 0);
  }
  Ok(())
}

#[test]
#[cfg(unix)]
fn child_deadline_reaps_process() -> Result<()> {
  let mut runner = isolate::Runner::new()?;
  let error = runner
    .run(
      Command::new("sh").args(["-c", "exec sleep 30"]),
      Duration::from_millis(10),
    )
    .unwrap_err();
  assert!(error.to_string().contains("timed out"), "{error}");
  assert_eq!(
    runner.run(
      Command::new("sh").args(["-c", "printf recovered"]),
      Duration::from_secs(2)
    )?,
    b"recovered"
  );
  Ok(())
}

#[test]
#[cfg(unix)]
fn child_failures_reset_output() -> Result<()> {
  let mut runner = isolate::Runner::new()?;
  for text in ["long first output", "x"] {
    assert_eq!(
      runner.run(
        Command::new("sh").args(["-c", &format!("printf '{text}'")]),
        Duration::from_secs(2)
      )?,
      text.as_bytes()
    );
  }
  let error = runner
    .run(
      Command::new("sh").args(["-c", "printf broken >&2; exit 3"]),
      Duration::from_secs(2),
    )
    .unwrap_err();
  assert!(error.to_string().contains("broken"));
  Ok(())
}
