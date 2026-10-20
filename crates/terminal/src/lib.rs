pub mod caps;
pub mod clipboard;
mod diacritics;
pub mod dnd;
pub mod gate;
pub mod graphics;
pub mod inline;
pub mod input;
pub mod painter;
pub mod payload;
pub mod spool;
pub mod style;
pub mod terminal;
pub mod tmux_observer;
use vpdf_model::{cache, text};
pub use vpdf_protocol::decode;
pub type Result<T> = anyhow::Result<T>;

const MIB: usize = 1 << 20;
