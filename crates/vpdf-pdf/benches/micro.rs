use anyhow::Result;
use mupdf::IRect;
use std::hint::black_box;
use vpdf_model::geometry::{page_matrix, transform};
use vpdf_model::text::PageId;
use vpdf_pdf::document::{Document, Handle};
#[path = "../../../benches/support/corpus.rs"]
mod corpus;
#[path = "../../../benches/support/mod.rs"]
mod support;
use corpus::corpus;
use support::measure;

fn scan_whole() -> Result<()> {
  let mut doc = Document::open(&corpus("ocr.pdf")?, "")?;
  let bounds = doc.pages[0].bounds;
  let matrix = page_matrix(bounds, 900, 0);
  let rect = transform(bounds, &matrix);
  let area = IRect::new(0, 0, rect.x1.ceil() as i32, rect.y1.ceil() as i32);
  let list = doc.display(PageId(0))?;
  measure("tiles::draw whole scan", || {
    drop(black_box(
      vpdf_pdf::raster::draw(&list, &matrix, area).expect("scan raster"),
    ));
  })
}

fn pack_read() -> Result<()> {
  use vpdf_pdf::content_cache::{Pack, Records, alias, save};
  let path = corpus("book.pdf")?;
  let mut doc = Document::open(&path, "")?;
  let pages = doc.pages.len();
  let mut records = Records::new(pages);
  for page in (0..pages).map(PageId) {
    let text = doc.text(page)?;
    records.add(page, &vpdf_search::search::Index::new(page, &text))?;
  }
  let root = tempfile::tempdir()?;
  let alias = alias(&mut std::fs::File::open(&path)?)?;
  save(root.path(), &alias, &records, None)?;
  let pack = Pack::open(root.path(), &alias)
    .ok_or_else(|| anyhow::anyhow!("pack missing"))?;
  measure(&format!("Pack::index {pages} pages"), || {
    for page in (0..pages).map(PageId) {
      drop(black_box(pack.index(page).expect("page index")));
    }
  })?;
  let all: Vec<PageId> = (0..pages).map(PageId).collect();
  measure(&format!("Pack::indexes {pages} pages"), || {
    let indexes =
      pack.indexes(&all, usize::MAX, vpdf_pdf::jobs::Config::default().helpers);
    assert_eq!(indexes.len(), pages);
    drop(black_box(indexes));
  })
}

fn cold_page() -> Result<()> {
  for (name, page) in [("paper.pdf", 1), ("book.pdf", 99)] {
    let mut doc = Document::open(&corpus(name)?, "")?;
    measure(&format!("cold text {name}"), || {
      doc.trim();
      drop(black_box(doc.text(PageId(page)).expect("text")));
    })?;
    measure(&format!("cold text+links {name}"), || {
      doc.trim();
      drop(black_box(doc.links(PageId(page)).expect("links")));
    })?;
    // Seeding retains text and drops links.
    let text = doc.text(PageId(page))?;
    measure(&format!("links, text read {name}"), || {
      doc.seed(PageId(page), std::sync::Arc::clone(&text));
      drop(black_box(doc.links(PageId(page)).expect("links")));
    })?;
  }
  Ok(())
}

fn page_list() -> Result<()> {
  let doc = Document::open(&corpus("book.pdf")?, "")?;
  measure("display list book p100", || {
    let page = doc.native.load_page(99);
    drop(black_box(
      page.and_then(|page| page.to_display_list(true)).expect("page list"),
    ));
  })
}

fn crop_margins() -> Result<()> {
  let mut doc = Document::open(&corpus("book.pdf")?, "")?;
  vpdf_pdf::crop::margins(&mut doc)?;
  measure("crop::margins book", || {
    drop(black_box(vpdf_pdf::crop::margins(&mut doc).expect("book margins")));
  })
}

fn crop_worker() -> Result<()> {
  use std::sync::atomic::AtomicBool;
  use vpdf_pdf::crop::Margins;
  let path = std::env::var_os("VPDF_CROP_INPUT")
    .map_or_else(|| corpus("book.pdf"), |path| Ok(path.into()))?;
  let mut doc = Document::open(&path, "")?;
  let handle = Handle::open(&path, "")?;
  let lists = doc.lists();
  let stop = AtomicBool::new(false);
  let mut pages: Vec<_> =
    (0..24).map(|index| PageId(index * (doc.pages.len() - 1) / 23)).collect();
  pages.dedup();
  let mut reference = Margins::default();
  reference.update(&handle, &pages, &[], &lists, &stop)?;
  let expected: Vec<_> =
    pages.iter().map(|page| reference.get(*page)).collect();
  measure("crop::batch interpreted lists", || {
    let mut margins = Margins::default();
    margins.update(&handle, &pages, &[], &lists, &stop).expect("crop batch");
    assert!(
      pages
        .iter()
        .zip(&expected)
        .all(|(page, crop)| margins.get(*page) == *crop)
    );
    black_box(margins);
  })?;
  let page = PageId(doc.pages.len() / 2);
  doc.display(page)?;
  let mut expected = reference.clone();
  expected.update(&handle, &[page], &[page], &lists, &stop)?;
  measure("crop::page warm list", || {
    let mut margins = reference.clone();
    margins
      .update(&handle, &[page], &[page], &lists, &stop)
      .expect("crop page");
    assert_eq!(margins.get(page), expected.get(page));
    black_box(margins);
  })
}

fn document_open() -> Result<()> {
  let path = corpus("book.pdf")?;
  measure("Document::open book", || {
    drop(black_box(Document::open(&path, "").expect("book document")));
  })
}

fn label_scan() -> Result<()> {
  let path = corpus("book.pdf")?;
  measure("Document::open + label miss", || {
    let doc = Document::open(&path, "").expect("corpus PDF");
    black_box(doc.page_number(black_box("missing")));
  })
}

fn handle_open() -> Result<()> {
  let path = corpus("book.pdf")?;
  measure("Handle::open book", || {
    drop(black_box(Handle::open(&path, "").expect("book handle")));
  })
}

fn handle_bounds() -> Result<()> {
  let doc = Handle::open(&corpus("book.pdf")?, "")?;
  measure("Handle::bounds book p200", || {
    black_box(doc.bounds(PageId(199)).expect("page bounds"));
  })
}

fn figures() -> Result<()> {
  let mut doc = Document::open(&corpus("paper.pdf")?, "")?;
  let page = PageId(2);
  doc.figures(page)?;
  measure("Document::figures warm", || {
    black_box(doc.figures(black_box(page)).expect("cached figures"));
  })?;
  Ok(())
}

fn worker() -> Result<()> {
  use std::sync::mpsc;
  use std::time::Duration;
  use vpdf_model::tile::TileKey;
  use vpdf_pdf::jobs::render::{RenderWorker, Sink};
  let path = corpus("paper.pdf")?;
  let mut doc = Document::open(&path, "")?;
  let keys = [0, 128].map(|x| TileKey {
    scale: 1024,
    x,
    y: 128,
    width: 128,
    height: 128,
    ..TileKey::default()
  });
  let expected = keys
    .iter()
    .map(|&key| vpdf_pdf::raster::raster(&mut doc, key))
    .collect::<Result<Vec<_>>>()?;
  let render = |key| -> Result<_> {
    let (send, receive) = mpsc::channel();
    let sink = Sink::new(move |result| send.send(result).is_ok());
    let worker = RenderWorker::new(&path, "", doc.lists(), sink);
    worker.request(&[key], 1);
    let tile = receive
      .recv_timeout(Duration::from_secs(10))?
      .map_err(anyhow::Error::msg)?;
    anyhow::ensure!(tile.key == key, "Unexpected worker tile");
    Ok(tile.pixels)
  };
  for (&key, pixels) in keys.iter().zip(&expected) {
    anyhow::ensure!(render(key)? == *pixels, "Worker pixels differ");
  }
  measure("RenderWorker::first tiles warm lists", || {
    for key in keys {
      black_box(render(key).expect("worker tile"));
    }
  })
}

fn main() -> Result<()> {
  support::run(&[
    ("scan whole", scan_whole),
    ("pack read", pack_read),
    ("cold page", cold_page),
    ("page list", page_list),
    ("crop margins", crop_margins),
    ("crop worker", crop_worker),
    ("document open", document_open),
    ("label scan", label_scan),
    ("handle open", handle_open),
    ("handle bounds", handle_bounds),
    ("figures", figures),
    ("worker readiness", worker),
  ])
}
