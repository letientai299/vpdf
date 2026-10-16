pub mod cache;
pub mod document;
pub mod search;
pub mod text;

pub type Result<T> = anyhow::Result<T>;
pub mod keys;
pub mod motion;
pub mod reader;
pub mod selection;
