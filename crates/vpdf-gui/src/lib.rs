#[cfg(target_os = "macos")]
mod collection;
pub mod fonts;
#[cfg(feature = "bench-events")]
pub mod inject;
pub mod input;
pub mod paint;
#[cfg(test)]
#[path = "../../../tests/support/screen.rs"]
pub mod screen;
mod style;
pub mod trace;
mod upload;
