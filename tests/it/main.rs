mod annotations;
mod app;
mod bench_stats;
mod chrome;
mod cli;
mod common;
mod corpus;
mod cursor;
mod cursorline;
mod definitions;
mod drop;
mod encrypted;
mod figures;
#[path = "../support/fixtures.rs"]
mod fixtures;
mod focus;
#[path = "../support/app.rs"]
mod harness;
#[path = "../../src/input.rs"]
mod input;
mod input_adapter;
mod jumps;
mod keys;
#[path = "../support/kitty.rs"]
mod kitty;
mod modes;
mod mouse;
mod objects;
mod outline;
mod paragraphs;
#[path = "../support/presentation.rs"]
pub mod presentation;
mod preview;
mod rendering;
mod sections;
mod session;
mod textlinks;
mod textpages;
mod ui_transport;
mod untrusted;
mod wheel;
mod workers;
