pub type Result<T> = anyhow::Result<T>;
use vpdf_model::text;
pub mod annotations;
pub mod chrome;
pub mod clues;
pub mod commands;
pub mod context;
pub mod figures;
pub mod find;
pub mod grid;
mod help;
pub mod hints;
pub mod home;
pub mod input;
pub mod keys;
pub mod layout;
mod line;
pub mod overlays;
pub mod overview;
pub mod page;
pub mod paint;
pub mod panel;
pub mod perf;
pub mod pointer;
pub mod preview;
pub mod prompt;
pub mod reading;
pub mod region;
pub mod scrollbar;
pub mod source;
pub mod split;
pub mod text_view;
pub mod theme;
pub mod window;

#[cfg(test)]
extern crate self as vpdf_ui;
#[cfg(test)]
#[path = "../../../tests/support/presentation.rs"]
pub mod presentation;
