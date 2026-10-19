//! Vim-style reading semantics over page text: cursor, motions, text
//! objects, selections, registers, and jumps.
pub mod motion;
pub mod objects;
mod paragraph;
pub mod reader;
mod selection;

pub type Result<T> = anyhow::Result<T>;
