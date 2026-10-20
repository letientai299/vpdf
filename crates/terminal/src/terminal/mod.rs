use crate::Result;
use crate::caps::{Caps, Cell, Host, Support};
use crate::input::Keyboard;
use anyhow::Context;
use crossterm::terminal::WindowSize;
use crossterm::{cursor, event, execute, terminal};
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once, PoisonError};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;
use vpdf_model::geometry::Screen;
use vpdf_protocol::decode::Size;

pub struct Terminal;

pub use lifecycle::{
  capture_mouse, defer_resize, images_lost, mouse, notice, resize_deferred,
  restore, restore_on_panic, resume, suspend,
};

use lifecycle::{FOCUS, reports};

pub use pointer::{Shape, claim_pointer, hand_over, point};

use pointer::shapes;

pub use metrics::{NO_CELL, cell, remeasure, resized, size};

pub(crate) use metrics::screen_with;

use metrics::{PROVOKED, note};

pub use tmux::{
  Cause, Sight, View, focus_reports, pane_view, sight, tmux, tmux_display,
  tmux_set,
};

pub(crate) use tmux::{VIEW, view};

pub use output::{
  At, HIDE_CURSOR, Transport, clean, clear, clipped, frame_end, frame_start,
  graphics, line, passthrough, placement, set_title, transmit,
};

mod lifecycle;
mod metrics;
mod output;
mod pointer;
#[cfg(test)]
mod tests;
mod tmux;
