use clap::{Parser, Subcommand};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use vpdf::{Result, document::Document, search, text::PageId};

#[derive(Parser)]
#[command(version, about)]
struct Args {
  #[command(subcommand)]
  command: Option<Command>,
  #[arg(value_name = "PDF")]
  file: Option<PathBuf>,
  #[arg(long)]
  text: bool,
  #[arg(long, global = true)]
  password_stdin: bool,
  #[arg(long, default_value_t = 1)]
  page: usize,
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
}

fn run(args: Args) -> Result<()> {
  let mut password = String::new();
  if args.password_stdin {
    io::stdin().lock().read_line(&mut password)?;
    password = password.trim_end_matches(['\n', '\r']).to_owned();
  }
  let mut out = io::BufWriter::new(io::stdout());
  let Some(command) = args.command else {
    let file = args.file.ok_or_else(|| anyhow::anyhow!("PDF path required"))?;
    let text = args.text
      || std::env::var_os("KITTY_WINDOW_ID").is_none()
        && std::env::var("TERM").is_ok_and(|term| !term.contains("kitty"));
    return vpdf::app::view(&file, &password, text, args.page);
  };
  match command {
    Command::Text { file, page } => {
      anyhow::ensure!(page > 0, "Page numbers start at one");
      let mut doc = Document::open(&file, &password)?;
      writeln!(out, "{}", doc.text(PageId(page - 1))?.plain())?;
    }
    Command::Info { file } => {
      let doc = Document::open(&file, &password)?;
      writeln!(
        out,
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
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
    eprintln!("vpdf: {error:#}");
    std::process::exit(1);
  }
}
