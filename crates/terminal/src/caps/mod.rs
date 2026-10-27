//! Startup capabilities, including the outer clipboard.
use crate::Result;
use std::fmt;
use std::io;
use std::sync::OnceLock;
use std::time::Duration;
use vpdf_protocol::decode::{ModeState, Reply};

pub use types::{
  Caps, Cell, Graphics, Host, Lack, Passthrough, Profile, Support, Tmux, WAIT,
};

use types::QUERY;

pub use keyboard::KeyLack;

pub use probe::{
  allow_passthrough, ask_cell, get, placeholders, probe, reset_passthrough,
  tmux_now, under_tmux,
};

pub(crate) use probe::{TMUX_FORMAT, tmux};

use probe::{PASSTHROUGH, flags, is, join};

#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;

mod keyboard;
mod probe;
mod types;

pub(crate) fn palette() -> vpdf_model::theme::Palette {
  get().map_or_default(Caps::palette)
}
