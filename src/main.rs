use clap::{Parser, Subcommand};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use vpdf::{Result, document::Document, search, text::PageId};

#[derive(Parser)]
#[command(version, about)]
struct Args {
  #[command(subcommand)]
  command: Option<Command>,
  /// Documents to open as tabs.
  #[arg(value_name = "PDF")]
  files: Vec<PathBuf>,
  #[arg(long)]
  text: bool,
  #[arg(long, global = true)]
  password_stdin: bool,
  /// Page to open instead of the saved position.
  #[arg(long)]
  page: Option<usize>,
  /// Frames per second while scrolling; set the display's refresh rate.
  #[arg(
    long,
    default_value_t = 120,
    value_parser = clap::value_parser!(u32).range(30..=480)
  )]
  fps: u32,
}

#[derive(Subcommand)]
enum Command {
  /// Extract page text.
  Text {
    file: PathBuf,
    #[arg(long, default_value_t = 1)]
    page: usize,
  },
  /// Inspect document metadata.
  Info { file: PathBuf },
  /// Search document text.
  Search { file: PathBuf, query: String },
  /// Print a page as an image in Kitty, else as text; sized for
  /// `fzf --preview` through `FZF_PREVIEW_COLUMNS` and `FZF_PREVIEW_LINES`.
  Preview {
    file: PathBuf,
    #[arg(long, default_value_t = 1)]
    page: usize,
  },
}

fn kitty() -> bool {
  std::env::var_os("KITTY_WINDOW_ID").is_some()
    || std::env::var("TERM").is_ok_and(|term| term.contains("kitty"))
}

/// Preview area from fzf, else the terminal, else 80x24.
fn preview_size() -> (u16, u16) {
  let read = |name| std::env::var(name).ok().and_then(|v| v.parse().ok());
  match (read("FZF_PREVIEW_COLUMNS"), read("FZF_PREVIEW_LINES")) {
    (Some(columns), Some(lines)) => (columns, lines),
    _ => crossterm::terminal::size().unwrap_or((80, 24)),
  }
}

fn run(args: Args) -> Result<()> {
  let mut password = String::new();
  if args.password_stdin {
    io::stdin().lock().read_line(&mut password)?;
    password = password.trim_end_matches(['\n', '\r']).to_owned();
  }
  let mut out = io::BufWriter::new(io::stdout());
  let Some(command) = args.command else {
    let launch = vpdf::session::Launch {
      text: args.text || !kitty(),
      page: args.page,
      fps: args.fps,
    };
    return vpdf::session::view(&args.files, &password, &launch);
  };
  match command {
    Command::Text { file, page } => {
      anyhow::ensure!(page > 0, "Page numbers start at one");
      let mut doc = Document::open(&file, &password)?;
      writeln!(out, "{}", doc.text(PageId(page - 1))?.plain())?;
    }
    Command::Preview { file, page } => {
      anyhow::ensure!(page > 0, "Page numbers start at one");
      let mut doc = Document::open(&file, &password)?;
      let page = PageId((page - 1).min(doc.pages.len() - 1));
      if kitty() {
        vpdf::inline::print(&mut doc, page, preview_size(), &mut out)?;
      } else {
        writeln!(out, "{}", doc.text(page)?.plain())?;
      }
    }
    Command::Info { file } => {
      let mut doc = Document::open(&file, &password)?;
      doc.load_outline()?;
      let metadata: serde_json::Map<_, _> =
        doc.metadata().into_iter().map(|(k, v)| (k, v.into())).collect();
      writeln!(
        out,
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
          "metadata": metadata,
          "pages": doc.pages, "outline": doc.outline,
        }))?
      )?;
    }
    Command::Search { file, query } => {
      let mut doc = Document::open(&file, &password)?;
      let pattern = search::compile(&query, false)?;
      let mut remaining = 10_000;
      for page in 0..doc.pages.len() {
        let text = doc.text(PageId(page))?;
        let index = search::Index::new(PageId(page), &text);
        let hits = index.matches(&pattern, remaining.min(1000))?;
        remaining -= hits.len();
        for hit in hits {
          writeln!(out, "{}", serde_json::to_string(&hit)?)?;
        }
        if remaining == 0 {
          break;
        }
      }
    }
  }
  out.flush()?;
  Ok(())
}

fn main() {
  if let Err(error) = run(Args::parse()) {
    if error
      .downcast_ref::<io::Error>()
      .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
    {
      return;
    }
    // MuPDF errors quote document bytes.
    let error: String = vpdf::text::printable(&format!("{error:#}")).collect();
    eprintln!("vpdf: {error}");
    std::process::exit(1);
  }
}
