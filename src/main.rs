use clap::Parser;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use vpdf::{Result, inline};
use vpdf_model::text::PageId;
use vpdf_pdf::document::Document;

macro_rules! about {
  () => {
    concat!(env!("CARGO_PKG_DESCRIPTION"), "\n", env!("CARGO_PKG_REPOSITORY"))
  };
}

#[derive(Parser)]
#[command(
  version = concat!(env!("VPDF_VERSION"), "\n", about!()),
  about = about!()
)]
struct Args {
  /// Documents to open as tabs.
  #[arg(value_name = "PDF")]
  files: Vec<PathBuf>,
  /// Page to open instead of the saved position.
  #[arg(long)]
  page: Option<usize>,
}

/// Preview area from fzf, else the terminal, else 80x24.
fn preview_size() -> (u16, u16) {
  let read = |name| std::env::var(name).ok().and_then(|v| v.parse().ok());
  match (read("FZF_PREVIEW_COLUMNS"), read("FZF_PREVIEW_LINES")) {
    (Some(columns), Some(lines)) => (columns, lines),
    _ => crossterm::terminal::size().unwrap_or((80, 24)),
  }
}

/// Prints the first page of `file` for the picker's fzf.
fn preview(file: &Path, graphics: inline::Graphics) -> Result<()> {
  let mut out = io::BufWriter::new(io::stdout());
  let mut doc = Document::open(file, "")?;
  match graphics {
    inline::Graphics::Kitty => {
      inline::print(&mut doc, PageId(0), preview_size(), &mut out)?;
    }
    inline::Graphics::Text => {
      writeln!(out, "{}", doc.text(PageId(0))?.plain())?;
    }
  }
  out.flush()?;
  Ok(())
}

fn run(args: &Args) -> Result<()> {
  if let Some(graphics) = inline::Graphics::exported() {
    let [file] = &args.files[..] else {
      anyhow::bail!("Preview takes one PDF");
    };
    return preview(file, graphics);
  }
  vpdf::session::view(&args.files, args.page)
}

fn main() {
  if let Err(error) = run(&Args::parse()) {
    if error
      .downcast_ref::<io::Error>()
      .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
    {
      return;
    }
    let text = format!("{error:#}");
    // The gate names each missing requirement on its own line.
    let lines = if error.is::<vpdf::gate::Unsupported>() {
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
