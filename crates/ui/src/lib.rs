pub type Result<T> = anyhow::Result<T>;
use vpdf_model::text;
pub mod annotations;
pub mod bindings;
pub mod chrome;
pub mod citations;
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
pub mod reference;
pub mod region;
pub mod scrollbar;
pub mod source;
pub mod split;
pub mod text_view;
pub mod theme;
pub mod topics;
pub mod window;

#[cfg(test)]
pub(crate) mod testing;

#[cfg(test)]
extern crate self as vpdf_ui;

#[cfg(test)]
#[path = "../../../tests/support/fixtures.rs"]
pub(crate) mod fixtures;
