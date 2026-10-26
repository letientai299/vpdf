use clap::Parser;
use std::io::{self, Write};
use std::path::PathBuf;
use vpdf::{Result, inline};
use vpdf_model::reference::{Operation, Reference};
use vpdf_model::text::PageId;
use vpdf_pdf::document::Document;

#[derive(Parser)]
#[command(
  version = vpdf::version::BUILD,
  about = vpdf::version::ABOUT
)]
struct Args {
  #[command(flatten)]
  config: vpdf::config::Options,
  /// PDF paths or file URIs, e.g. book.pdf#page=2.
  #[arg(value_name = "PDF")]
  files: Vec<PathBuf>,
}

/// Preview area from fzf, else the terminal, else 80x24.
fn preview_size() -> (u16, u16) {
  let read = |name| std::env::var(name).ok().and_then(|v| v.parse().ok());
  match (read("FZF_PREVIEW_COLUMNS"), read("FZF_PREVIEW_LINES")) {
    (Some(columns), Some(lines)) => (columns, lines),
    _ => crossterm::terminal::size().unwrap_or((80, 24)),
  }
}

/// Prints a referenced page for the picker.
fn preview(reference: &Reference, graphics: inline::Graphics) -> Result<()> {
  let mut out = io::BufWriter::new(io::stdout());
  let mut doc = Document::open(&reference.path, "")?;
  let mut page = PageId(0);
  for operation in &reference.operations {
    match operation {
      Operation::Page(number) => {
        anyhow::ensure!(
          number.get() <= doc.pages.len(),
          "Page outside document"
        );
        page = PageId(number.get() - 1);
      }
      _ => anyhow::bail!("Preview supports only page targets"),
    }
  }
  match graphics {
    inline::Graphics::Kitty => {
      inline::print(&mut doc, page, preview_size(), &mut out)?;
    }
    inline::Graphics::Text => {
      writeln!(out, "{}", doc.text(page)?.plain())?;
    }
  }
  out.flush()?;
  Ok(())
}

fn run(args: &Args) -> Result<()> {
  if args.config.config_example {
    return vpdf::config::print_example();
  }
  let graphics = inline::Graphics::exported();
  let references = if graphics.is_some()
    && std::env::var_os(inline::PATH_VAR).as_deref()
      == Some(std::ffi::OsStr::new("1"))
  {
    args.files.iter().cloned().map(Reference::from).collect()
  } else {
    args
      .files
      .iter()
      .map(|file| Reference::parse(file.as_os_str()))
      .collect::<std::result::Result<Vec<_>, _>>()?
  };
  if let Some(graphics) = graphics {
    let [file] = &references[..] else {
      anyhow::bail!("Preview takes one PDF");
    };
    return preview(file, graphics);
  }
  vpdf::session::view(&references, args.config.load())
}

fn main() {
  if let Err(error) = run(&vpdf::version::parse::<Args>(true)) {
    if error
      .downcast_ref::<io::Error>()
      .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
    {
      return;
    }
    let text = format!("{error:#}");
    // The gate names each missing requirement on its own line.
    let lines = if error.is::<vpdf_terminal::gate::Unsupported>() {
      text.lines().collect()
    } else {
      vec![text.as_str()]
    };
    for line in lines {
      // MuPDF errors quote document bytes.
      let line: String = vpdf_model::text::printable(line).collect();
      eprintln!("vpdf: {line}");
    }
    std::process::exit(1);
  }
}
