mod autolink;
pub mod crop;
pub mod document;
pub mod figures;
mod fonts;
mod prime;
pub mod raster;
mod scheduling;
pub mod wraps;

use vpdf_model::{cache, text};
use vpdf_search::search;
pub type Result<T> = anyhow::Result<T>;

pub mod content_cache;
pub mod jobs;
#[cfg(test)]
use jobs::Wake;
#[cfg(test)]
#[path = "../../../tests/support/encryption.rs"]
mod encryption;
#[cfg(test)]
#[path = "../../../tests/support/native.rs"]
mod fixtures;
