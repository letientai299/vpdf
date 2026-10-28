//! Full-session scenarios without a window.

use clap::Parser;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use vpdf::Result;
use vpdf::scenario::Scenario;
use vpdf::trace::Log;

#[path = "../support/hash.rs"]
mod hash;
#[path = "../../../src/provenance.rs"]
mod provenance;
#[path = "drive/runner.rs"]
mod runner;
#[path = "../support/timing.rs"]
mod stats;

#[derive(Parser)]
#[command(after_help = vpdf::scenario::Expectation::HELP)]
struct Args {
  /// Scenario files with verified PDF headers.
  #[arg(required = true)]
  scenarios: Vec<PathBuf>,
  /// Results directory in the host cache.
  #[arg(long)]
  output: Option<PathBuf>,
  #[arg(
    long, default_value_t = 1,
    value_parser = clap::value_parser!(u32).range(1..=100)
  )]
  rounds: u32,
  #[arg(
    long, default_value_t = 120,
    value_parser = clap::value_parser!(u32).range(1..=600)
  )]
  timeout: u32,
  /// Directory for standard corpus PDFs.
  #[arg(long)]
  corpus: Option<PathBuf>,
  /// Directory for large corpus PDFs.
  #[arg(long, default_value = "tmp/huge")]
  huge: PathBuf,
  #[arg(long, hide = true)]
  bench: bool,
  #[arg(long, hide = true)]
  child_round: Option<u32>,
}

#[derive(Serialize)]
struct InputSource {
  path: PathBuf,
  sha256: String,
}

#[derive(Serialize)]
struct Report<'a> {
  schema: u8,
  pid: u32,
  backend: &'static str,
  revision: &'static str,
  source_dirty: Option<bool>,
  runtime_revision: Option<String>,
  runtime_patch: Option<String>,
  build_revision: &'static str,
  build_dirty: &'static str,
  build_patch: &'static str,
  features: &'static str,
  binary_sha256: String,
  scenario: PathBuf,
  scenario_sha256: String,
  inputs: Vec<InputSource>,
  machine: String,
  load_before: f64,
  cpu_threads: usize,
  screen: (u16, u16, u16, u16),
  window: (u32, u32),
  crop: bool,
  cache: &'static str,
  round: u32,
  trace: PathBuf,
  summary: runner::Summary,
  waits: &'a [runner::WaitRecord],
}

fn output(args: &Args) -> Result<PathBuf> {
  if let Some(path) = &args.output {
    return Ok(path.clone());
  }
  runner::results("headless")
}

fn sources(scenario: &Scenario, args: &Args) -> Result<Vec<InputSource>> {
  scenario
    .sources()
    .iter()
    .map(|source| {
      let name = format!("{}.pdf", source.name());
      let corpus = args
        .corpus
        .clone()
        .or_else(|| std::env::var_os("VPDF_CORPUS").map(PathBuf::from))
        .ok_or_else(|| anyhow::anyhow!("Scenario corpus path missing"))?;
      let candidate = corpus.join(&name);
      let path =
        if candidate.is_file() { candidate } else { args.huge.join(name) };
      let path = path.canonicalize()?;
      let sha256 = hash::digest(&std::fs::read(&path)?);
      anyhow::ensure!(
        sha256 == source.sha256(),
        "Scenario PDF hash mismatch: {}",
        path.display()
      );
      Ok(InputSource { path, sha256 })
    })
    .collect()
}

fn run(path: &Path, args: &Args, root: &Path, round: u32) -> Result<()> {
  let bytes = std::fs::read(path)?;
  let scenario = Scenario::parse(std::str::from_utf8(&bytes)?)?;
  let inputs = sources(&scenario, args)?;
  let name = path.file_stem().unwrap_or_default().to_string_lossy();
  let directory = root.join(format!("{name}-{round}"));
  anyhow::ensure!(!directory.exists(), "Scenario output already exists");
  std::fs::create_dir_all(&directory)?;
  let binary_sha256 = hash::digest(&std::fs::read(std::env::current_exe()?)?);
  let machine = String::from_utf8(
    std::process::Command::new("uname").arg("-a").output()?.stdout,
  )?
  .trim()
  .to_owned();
  anyhow::ensure!(!machine.is_empty(), "Machine identity unavailable");
  let runtime = provenance::Source::read(&std::env::current_dir()?);
  let load_before = sysinfo::System::load_average().one;
  let cpu_threads = std::thread::available_parallelism()?.get();
  let trace = directory.join("trace.jsonl");
  let log = Arc::new(Log::open(&trace)?);
  log.mark(std::time::Instant::now(), "startup")?;
  let screen = scenario.screen();
  let mut driver = runner::Driver::open(
    inputs.iter().map(|input| input.path.clone()).collect(),
    screen,
    log,
    Duration::from_secs(u64::from(args.timeout)),
  )?;
  driver.wait(&vpdf::scenario::Wait::Complete)?;
  anyhow::ensure!(
    driver
      .last
      .as_ref()
      .is_some_and(|frame| frame.metrics.observation.pages > 0),
    "Scenario documents did not open"
  );
  if scenario.crop() {
    let Some(vpdf::scenario::Step::Keys(keys)) =
      vpdf::scenario::Step::parse("key z c")?
    else {
      unreachable!()
    };
    driver.keys(&keys, 0)?;
    driver.wait(&vpdf::scenario::Wait::Idle)?;
  }
  for (index, step) in scenario.steps().iter().enumerate() {
    if driver.step(step, index + 1)?.is_break() {
      break;
    }
  }
  let (summary, waits) = driver.finish()?;
  let report = Report {
    schema: 2,
    pid: std::process::id(),
    backend: "headless",
    revision: vpdf::version::VERSION,
    source_dirty: runtime.as_ref().map(|s| s.dirty),
    runtime_revision: runtime.as_ref().map(|s| s.revision.clone()),
    runtime_patch: runtime.map(|s| s.patch),
    build_revision: vpdf::version::REVISION,
    build_dirty: vpdf::version::DIRTY,
    build_patch: vpdf::version::PATCH,
    features: "bench-events",
    binary_sha256,
    scenario: path.canonicalize()?,
    scenario_sha256: hash::digest(&bytes),
    inputs,
    machine,
    load_before,
    cpu_threads,
    screen: (
      screen.columns,
      screen.rows,
      screen.cell_width,
      screen.cell_height,
    ),
    window: scenario.window(),
    crop: scenario.crop(),
    cache: "OS warm after hashing; fresh session/state; VPDF_CACHE=off",
    round,
    trace,
    summary,
    waits: &waits,
  };
  let json = serde_json::to_string_pretty(&report)?;
  std::fs::write(directory.join("report.json"), &json)?;
  println!("{json}");
  Ok(())
}

#[path = "../support/workspace.rs"]
mod workspace;

fn main() -> Result<()> {
  workspace::enter()?;
  let args = Args::parse();
  anyhow::ensure!(
    std::env::var("VPDF_CACHE").is_ok_and(|value| value == "off"),
    "Headless benchmark requires VPDF_CACHE=off"
  );
  let root = output(&args)?;
  if let Some(round) = args.child_round {
    anyhow::ensure!(args.scenarios.len() == 1, "Child requires one scenario");
    return run(&args.scenarios[0], &args, &root, round);
  }
  for round in 1..=args.rounds {
    for scenario in &args.scenarios {
      let mut command = std::process::Command::new(std::env::current_exe()?);
      command
        .arg(scenario)
        .arg("--output")
        .arg(&root)
        .arg("--child-round")
        .arg(round.to_string())
        .arg("--timeout")
        .arg(args.timeout.to_string())
        .arg("--huge")
        .arg(&args.huge);
      if let Some(corpus) = &args.corpus {
        command.arg("--corpus").arg(corpus);
      }
      anyhow::ensure!(command.status()?.success(), "Scenario process failed");
    }
  }
  Ok(())
}
