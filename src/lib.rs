pub use vpdf_model::cache;
pub use vpdf_pdf::document;
mod editor;
pub use vpdf_model::text;
pub use vpdf_search::search;

pub type Result<T> = anyhow::Result<T>;
pub mod animation;
mod annotations;
pub mod app;
mod canvas;
mod chrome;
mod clipboard;
mod clues;
pub mod cmdline;
mod commands;
mod context;
pub mod crop;
mod definition;
pub mod dnd;
pub use vpdf_pdf::figures;
mod find;
#[cfg(test)]
use vpdf_pdf::jobs::Wake;
#[cfg(test)]
#[path = "../tests/support/native.rs"]
mod fixtures;
pub mod fzf;
pub mod gate;
mod gestures;
mod hangup;
mod hints;
pub mod home;
pub mod inline;
mod interaction;
mod jumps;
mod lifecycle;
pub mod links;
mod marks;
pub mod ocr;
pub mod outline;
mod overlays;
mod overview;
mod pace;
pub mod panel;
mod password;
pub mod pdf_text;
mod perf;
mod preview;
pub mod raster;
pub mod recent;
pub use vpdf_search::reference;
mod region;
mod scrollbar;
mod section;
pub mod session;
pub mod solo;
pub mod state;
mod statusline;
mod text_canvas;
pub mod tiles;
pub mod trace;
pub mod watch;
use vpdf_pdf::wraps;

#[cfg(test)]
#[path = "../tests/support/reader.rs"]
mod reader_fixture;
#[cfg(test)]
pub(crate) use reader_fixture::SeedReader;
