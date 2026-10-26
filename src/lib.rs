pub type Result<T> = anyhow::Result<T>;
pub mod app;
mod canvas;
mod clipboard;
pub mod cmdline;
mod commands;
pub mod config;
mod context;
pub mod dnd;
mod find;
mod gestures;
pub mod home;
pub mod inline;
mod input;
pub mod jobs;
pub mod links;
pub mod manual;
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
mod references;
mod region;
pub mod scenario;
mod scrollbar;
pub mod session;
mod sidebar;
pub mod solo;
pub mod state;
mod status;
pub mod trace;
pub mod version;
mod view;
pub mod watch;

#[cfg(test)]
use vpdf_pdf::jobs::Wake;
#[cfg(test)]
#[path = "../tests/support/encryption.rs"]
mod encryption;
#[cfg(test)]
#[path = "../tests/support/native.rs"]
mod fixtures;
#[cfg(test)]
#[path = "../tests/support/kitty.rs"]
mod kitty;
#[cfg(test)]
#[path = "../tests/support/reader.rs"]
mod reader_fixture;
#[cfg(test)]
#[path = "../tests/support/screen.rs"]
mod screen;
#[cfg(test)]
pub(crate) use reader_fixture::SeedReader;

pub mod presentation;

#[cfg(test)]
mod provenance;

#[cfg(test)]
mod provenance_tests;

#[cfg(test)]
#[path = "../tests/support/references.rs"]
mod reference_fixture;
