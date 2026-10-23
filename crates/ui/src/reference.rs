use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum Source {
  #[default]
  Default,
  File(PathBuf),
  Command,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Setting {
  pub name: String,
  pub default: Value,
  pub effective: Value,
  pub source: Source,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Configuration {
  pub generation: u64,
  pub settings: Vec<Setting>,
  pub warnings: Vec<String>,
}
