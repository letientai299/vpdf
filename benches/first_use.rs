//! Application-cache first-use costs, excluding setup.
//!
//! Fresh processes isolate native workers and caches. Scene discovery warms
//! native internals before application caches are trimmed. Preparation is
//! synchronous and outside the input timer. Missing foreground tiles establish
//! pending work, not proof of native overlap. Frames go to memory; `ready_ms`
//! ends after input and a complete base view, not terminal painting.
//!
//! Cases without text or a usable link record skipped observations. Skip
//! eligibility precedes preparation, so one skipped child skips all modes.
//! Page indices are zero-based.
//! Text-cache weight excludes lists and native allocations.
//! Use --help for filters; preserve raw samples and input/build identities.
#[path = "support/corpus.rs"]
mod corpus;
#[path = "first_use/isolate.rs"]
mod isolate;
#[path = "first_use/sample.rs"]
mod sample;

use anyhow::Context;
use clap::{Arg, ArgAction, Command, value_parser};
use isolate::Runner;
use sample::{Interaction, Observation, Preparation, Request};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
use vpdf::Result;
use vpdf_pdf::document::Document;

fn options() -> clap::ArgMatches {
  Command::new("first_use")
    .about("Measure isolated application-cache first-use interactions")
    .arg(
      Arg::new("pdf")
        .long("pdf")
        .action(ArgAction::Append)
        .value_parser(value_parser!(PathBuf))
        .help("PDF path; repeat for multiple inputs"),
    )
    .arg(
      Arg::new("case")
        .long("case")
        .action(ArgAction::Append)
        .value_parser(value_parser!(Interaction))
        .help("selection, cursor, hover, figures, preview"),
    )
    .arg(
      Arg::new("samples")
        .long("samples")
        .default_value("31")
        .value_parser(value_parser!(u16).range(1..=1000))
        .help("Attempts per interaction and preparation"),
    )
    .arg(
      Arg::new("rendering")
        .long("rendering")
        .action(ArgAction::SetTrue)
        .help("Submit foreground rendering before each action"),
    )
    .arg(
      Arg::new("timeout")
        .long("timeout")
        .default_value("30")
        .value_parser(value_parser!(u16).range(1..))
        .help("Per-child deadline in seconds"),
    )
    .arg(Arg::new("sample").long("sample").hide(true))
    .arg(Arg::new("bench").long("bench").hide(true).action(ArgAction::SetTrue))
    .get_matches()
}

const MODES: [Preparation; 3] =
  [Preparation::Cold, Preparation::List, Preparation::Text];

struct Harness {
  executable: PathBuf,
  runner: Runner,
  timeout: Duration,
  rendering: bool,
}

impl Harness {
  fn observe(&mut self, request: &Request) -> Result<Observation> {
    let mut command = std::process::Command::new(&self.executable);
    command.arg("--sample").arg(serde_json::to_string(request)?);
    let value: Observation = self
      .runner
      .run(&mut command, self.timeout)
      .and_then(|bytes| Ok(serde_json::from_slice(bytes)?))
      .with_context(|| {
        format!(
          "Interaction failed: {} page {}",
          request.path.display(),
          request.page
        )
      })?;
    anyhow::ensure!(value.page == request.page, "Observation page differs");
    Ok(value)
  }

  fn observations(
    &mut self,
    path: &Path,
    name: Interaction,
    picks: &[usize],
  ) -> Result<[Vec<Observation>; 3]> {
    let mut observations: [Vec<Observation>; 3] =
      std::array::from_fn(|_| Vec::with_capacity(picks.len()));
    for (index, &page) in picks.iter().enumerate() {
      let mut skipped = false;
      for offset in 0..MODES.len() {
        let mode = (index + offset) % MODES.len();
        let request = Request {
          path: path.into(),
          name,
          page,
          prepared: MODES[mode],
          rendering: self.rendering,
        };
        let value = self.observe(&request)?;
        if value.milliseconds.is_none() {
          anyhow::ensure!(
            offset == 0,
            "Scene eligibility changed across modes"
          );
          skipped = true;
          break;
        }
        observations[mode].push(value);
      }
      if skipped {
        for values in &mut observations {
          values.push(Observation::skipped(page));
        }
      }
    }
    Ok(observations)
  }
}

fn main() -> Result<()> {
  let args = options();
  if let Some(request) = args.get_one::<String>("sample") {
    println!(
      "{}",
      serde_json::to_string(&sample::sample(&serde_json::from_str(request)?)?)?
    );
    return Ok(());
  }
  let paths: Vec<PathBuf> = if let Some(paths) = args.get_many::<PathBuf>("pdf")
  {
    paths.cloned().collect()
  } else if let Some(extra) = std::env::var_os("VPDF_EXTRA") {
    std::env::split_paths(&extra).collect()
  } else {
    vec![corpus::corpus("paper.pdf")?]
  };
  let interactions = args
    .get_many::<Interaction>("case")
    .map(|cases| cases.copied().collect::<Vec<_>>())
    .unwrap_or(vec![
      Interaction::Selection,
      Interaction::Cursor,
      Interaction::Hover,
      Interaction::Figures,
      Interaction::Preview,
    ]);
  let attempts = usize::from(*args.get_one::<u16>("samples").unwrap());
  let timeout =
    Duration::from_secs(u64::from(*args.get_one::<u16>("timeout").unwrap()));
  let rendering = args.get_flag("rendering");
  let mut harness = Harness {
    executable: std::env::current_exe()?,
    runner: Runner::new()?,
    timeout,
    rendering,
  };
  let mut seen = HashSet::new();
  for path in paths {
    let path = path.canonicalize()?;
    anyhow::ensure!(seen.insert(path.clone()), "Duplicate PDF input");
    let pages = Document::open(&path, "")?.pages.len();
    let picks = sample::plan(pages, attempts);
    for &name in &interactions {
      let observations = harness.observations(&path, name, &picks)?;
      for (prepared, samples) in MODES.iter().zip(observations) {
        println!(
          "{}",
          serde_json::json!({
            "document": path,
            "rendering": rendering,
            "case": name,
            "prepared": prepared,
            "attempts": attempts,
            "screen": [
              sample::PANE.columns, sample::PANE.rows,
              sample::PANE.cell_width, sample::PANE.cell_height,
            ],
            "samples": samples,
          })
        );
      }
    }
  }
  Ok(())
}
