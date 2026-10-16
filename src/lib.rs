pub mod cache;
pub mod document;
pub mod search;
pub mod text;

pub type Result<T> = anyhow::Result<T>;
pub mod app;
mod canvas;
pub mod graphics;
mod interaction;
pub mod keys;
pub mod layout;
pub mod motion;
pub mod reader;
pub mod render_worker;
pub mod selection;
pub mod terminal;
pub mod tiles;
