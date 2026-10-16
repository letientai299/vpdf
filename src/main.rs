use clap::{Parser, Subcommand};
use std::path::PathBuf;
use vpdf::{Result, document::Document, search, text::PageId};

#[derive(Parser)]
#[command(version, about)]
struct Args {
  #[command(subcommand)]
  command: Command,
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
  match args.command {
    Command::Text { file, page } => {
      anyhow::ensure!(page > 0, "Page numbers start at one");
      let mut doc = Document::open(&file, "")?;
      println!("{}", doc.text(PageId(page - 1))?.plain());
    }
    Command::Info { file } => {
      let doc = Document::open(&file, "")?;
      println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
          "pages": doc.pages, "outline": doc.outline,
        }))?
      );
    }
    Command::Search { file, query } => {
      let mut doc = Document::open(&file, "")?;
      let pattern = search::compile(&query, false)?;
      let mut remaining = 10_000;
      for page in 0..doc.pages.len() {
        let text = doc.text(PageId(page))?;
        let index = search::Index::new(PageId(page), &text);
        let hits = index.matches(&pattern, remaining.min(1000))?;
        remaining -= hits.len();
        for hit in hits {
          println!("{}", serde_json::to_string(&hit)?);
        }
        if remaining == 0 {
          break;
        }
      }
    }
  }
  Ok(())
}

fn main() {
  if let Err(error) = run(Args::parse()) {
    eprintln!("vpdf: {error:#}");
    std::process::exit(1);
  }
}
