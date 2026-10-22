pub mod query;
pub mod reference;
pub mod search;
#[cfg(test)]
#[path = "../../../tests/support/text.rs"]
mod text_fixture;

use vpdf_model::text;
type Result<T> = anyhow::Result<T>;
