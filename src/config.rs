use clap::Args;
use serde::Serialize;
use std::ffi::OsStr;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use vpdf_ui::layout::{MAX_ZOOM, MIN_ZOOM};

#[derive(Args, Debug, Default)]
pub struct Options {
  /// Read one configuration file.
  #[arg(long, value_name = "PATH", conflicts_with = "no_config")]
  pub config: Option<PathBuf>,
  /// Use built-in settings.
  #[arg(long)]
  pub no_config: bool,
  /// Print default configuration and exit.
  #[arg(long)]
  pub config_example: bool,
}

impl Options {
  pub fn load(&self) -> Loaded {
    self.load_from(
      std::env::var_os("XDG_CONFIG_HOME").as_deref(),
      std::env::var_os("HOME").as_deref(),
    )
  }

  fn load_from(&self, xdg: Option<&OsStr>, home: Option<&OsStr>) -> Loaded {
    if self.no_config {
      return Loaded::default();
    }
    if let Some(path) = &self.config {
      return load(path, true);
    }
    discover(xdg, home).map_or_else(Loaded::default, |path| load(&path, false))
  }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Settings {
  reading: Reading,
  ocr: Ocr,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct Reading {
  cursorline: bool,
  zoom: Zoom,
  inactive_brightness: Brightness,
}

impl Default for Reading {
  fn default() -> Self {
    Self {
      cursorline: true,
      zoom: Zoom(vpdf_ui::layout::DEFAULT_ZOOM),
      inactive_brightness: Brightness(0.8),
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(transparent)]
struct Brightness(f64);

impl TryFrom<f64> for Brightness {
  type Error = &'static str;

  fn try_from(value: f64) -> Result<Self, Self::Error> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
      Ok(Self(value))
    } else {
      Err("brightness outside supported range")
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(transparent)]
struct Zoom(f32);

impl TryFrom<f64> for Zoom {
  type Error = &'static str;

  fn try_from(value: f64) -> Result<Self, Self::Error> {
    if value.is_finite() && (MIN_ZOOM..=MAX_ZOOM).contains(&value) {
      Ok(Self(value as f32))
    } else {
      Err("zoom outside supported range")
    }
  }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
struct Ocr {
  language: Language,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct Language(String);

impl Default for Language {
  fn default() -> Self {
    Self("eng".into())
  }
}

impl TryFrom<String> for Language {
  type Error = &'static str;

  fn try_from(value: String) -> Result<Self, Self::Error> {
    if value.chars().any(char::is_control)
      || value.split('+').any(|part| part.trim().is_empty())
    {
      Err("invalid OCR language")
    } else {
      Ok(Self(value))
    }
  }
}

impl Settings {
  pub fn cursorline(&self) -> bool {
    self.reading.cursorline
  }

  pub fn zoom(&self) -> f32 {
    self.reading.zoom.0
  }

  pub fn language(&self) -> &str {
    &self.ocr.language.0
  }

  pub fn inactive_brightness(&self) -> f64 {
    self.reading.inactive_brightness.0
  }

  pub fn example() -> crate::Result<String> {
    Ok(toml::to_string_pretty(&Self::default())?)
  }
}

pub fn print_example() -> crate::Result<()> {
  let example = Settings::example()?;
  match io::stdout().lock().write_all(example.as_bytes()) {
    Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
    result => Ok(result?),
  }
}

#[derive(Debug, Default)]
pub struct Loaded {
  pub settings: Settings,
  pub warning: Option<String>,
}

fn discover(xdg: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
  xdg
    .map(Path::new)
    .filter(|path| path.is_absolute())
    .map(Path::to_owned)
    .or_else(|| {
      home
        .map(Path::new)
        .filter(|path| path.is_absolute())
        .map(|path| path.join(".config"))
    })
    .map(|path| path.join("vpdf/config.toml"))
}

fn load(path: &Path, explicit: bool) -> Loaded {
  match std::fs::read_to_string(path) {
    Ok(text) => parse(path, &text),
    Err(error) if !explicit && error.kind() == io::ErrorKind::NotFound => {
      Loaded::default()
    }
    Err(error) => finish(path, Settings::default(), &[error.to_string()]),
  }
}

fn parse(path: &Path, text: &str) -> Loaded {
  let value = match toml::from_str::<toml::Value>(text) {
    Ok(value) => value,
    Err(error) => {
      return finish(path, Settings::default(), &[error.to_string()]);
    }
  };
  let mut settings = Settings::default();
  let mut warnings = Vec::new();
  if let Some(root) = value.as_table() {
    for (section, value) in root {
      if !matches!(section.as_str(), "reading" | "ocr") {
        warnings.push(format!("unknown setting {section}"));
        continue;
      }
      let Some(table) = value.as_table() else {
        warnings.push(format!("{section}: expected table"));
        continue;
      };
      for (key, value) in table {
        let name = format!("{section}.{key}");
        let result = match name.as_str() {
          "reading.cursorline" => value
            .as_bool()
            .ok_or("expected boolean")
            .map(|value| settings.reading.cursorline = value),
          "reading.zoom" => value
            .clone()
            .try_into::<f64>()
            .map_err(|_| "expected number")
            .and_then(Zoom::try_from)
            .map(|value| settings.reading.zoom = value),
          "reading.inactive_brightness" => value
            .clone()
            .try_into::<f64>()
            .map_err(|_| "expected number")
            .and_then(Brightness::try_from)
            .map(|value| settings.reading.inactive_brightness = value),
          "ocr.language" => value
            .as_str()
            .ok_or("expected string")
            .and_then(|value| Language::try_from(value.to_owned()))
            .map(|value| settings.ocr.language = value),
          _ => Err("unknown setting"),
        };
        if let Err(error) = result {
          warnings.push(format!("{name}: {error}"));
        }
      }
    }
  }
  finish(path, settings, &warnings)
}

fn finish(path: &Path, settings: Settings, warnings: &[String]) -> Loaded {
  let warning = (!warnings.is_empty()).then(|| {
    let message = format!("Config {}: {}", path.display(), warnings.join("; "));
    vpdf_model::text::printable(&message).collect()
  });
  Loaded { settings, warning }
}

#[cfg(test)]
mod tests;
