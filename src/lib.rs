pub type Result<T> = anyhow::Result<T>;
pub mod app;
mod canvas;
mod clipboard;
pub mod cmdline;
mod commands;
mod context;
pub mod dnd;
mod find;
mod gestures;
pub mod home;
pub mod inline;
pub mod jobs;
pub mod links;
mod navigation;
mod overlays;
mod overview;
mod pace;
pub mod panel;
mod password;
pub mod pdf_text;
mod perf;
mod pointer;
mod preview;
pub mod programs;
pub mod raster;
pub mod recent;
mod region;
mod scrollbar;
pub mod session;
pub mod solo;
pub mod state;
mod status;
pub mod trace;
mod view;
pub mod watch;

#[cfg(test)]
use vpdf_pdf::jobs::Wake;
#[cfg(test)]
#[path = "../tests/support/native.rs"]
mod fixtures;
#[cfg(test)]
#[path = "../tests/support/reader.rs"]
mod reader_fixture;
#[cfg(test)]
pub(crate) use reader_fixture::SeedReader;
