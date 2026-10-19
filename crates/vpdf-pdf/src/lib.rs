mod autolink;
pub mod crop;
pub mod document;
pub mod figures;
mod fonts;
pub mod layout;
pub mod prime;
pub mod raster;
pub mod scheduling;
pub mod wraps;

use vpdf_model::{cache, text};
pub type Result<T> = anyhow::Result<T>;
