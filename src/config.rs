mod bindings;
mod reference;
mod theme;
use clap::Args;
use serde::Serialize;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
pub use theme::ThemeChoice;
use vpdf_ui::layout::{MAX_ZOOM, MIN_ZOOM};

pub(crate) fn reload_command(argument: &str) -> crate::Result<()> {
  anyhow::ensure!(argument.trim() == "reload", "Usage: :config reload");
  Ok(())
}

#[derive(Args, Debug, Default)]
pub struct Options {
  /// Read one configuration file.
  #[arg(short = 'u', long = "config", value_name = "PATH")]
  pub config: Option<PathBuf>,
}

impl Options {
  pub fn load(&self) -> Loaded {
    self.load_from(
      std::env::var_os("XDG_CONFIG_HOME").as_deref(),
      std::env::var_os("HOME").as_deref(),
    )
  }

  fn load_from(&self, xdg: Option<&OsStr>, home: Option<&OsStr>) -> Loaded {
    if let Some(path) = &self.config {
      let Ok(text) = std::fs::read_to_string(path) else {
        return Loaded::default();
      };
      let source = Source::file(path, true);
      let Source::File { path, .. } = &source else { unreachable!() };
      let mut loaded = parse_with(path, &text, &Settings::default());
      loaded.source = source;
      return loaded;
    }
    discover(xdg, home).map_or_else(Loaded::default, |path| {
      Source::file(&path, false).reload(&Settings::default())
    })
  }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Settings {
  appearance: Appearance,
  reading: Reading,
  ocr: Ocr,
  #[serde(rename = "keybindings")]
  bindings: vpdf_ui::bindings::Bindings,
  #[serde(skip)]
  reference: reference::Resolved,
}

// Provenance does not change setting values.
impl PartialEq for Settings {
  fn eq(&self, other: &Self) -> bool {
    self.appearance == other.appearance
      && self.reading == other.reading
      && self.ocr == other.ocr
      && self.bindings == other.bindings
  }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
struct Appearance {
  theme: ThemeChoice,
  ascii_glyphs: bool,
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
  pub fn colors(
    &self,
    system: vpdf_model::theme::Colors,
  ) -> vpdf_model::theme::Colors {
    self.appearance.theme.colors(system)
  }

  pub fn theme(&self) -> &ThemeChoice {
    &self.appearance.theme
  }

  pub(crate) fn set_theme(&mut self, theme: impl Into<ThemeChoice>) {
    self.appearance.theme = theme.into();
    self.reference.generation += 1;
    self
      .reference
      .sources
      .insert("appearance.theme".into(), vpdf_ui::reference::Source::Command);
  }

  pub fn bindings(&self) -> &vpdf_ui::bindings::Bindings {
    &self.bindings
  }

  fn apply(
    &mut self,
    name: &str,
    value: &toml::Value,
    previous: &Self,
    path: &Path,
  ) -> Result<(), String> {
    if name == "appearance.theme" {
      let theme = value
        .as_str()
        .ok_or_else(|| vpdf_model::theme::Theme::EXPECTED.to_owned())
        .and_then(|name| {
          ThemeChoice::load(name, path.parent().unwrap_or(Path::new(".")))
        });
      self.appearance.theme = theme.as_ref().cloned().unwrap_or_default();
      return theme.map(|_| ());
    }
    match name {
      "appearance.ascii_glyphs" => replace_setting(
        &mut self.appearance.ascii_glyphs,
        &previous.appearance.ascii_glyphs,
        value.as_bool().ok_or("expected boolean"),
      ),
      "reading.cursorline" => replace_setting(
        &mut self.reading.cursorline,
        &previous.reading.cursorline,
        value.as_bool().ok_or("expected boolean"),
      ),
      "reading.zoom" => replace_setting(
        &mut self.reading.zoom,
        &previous.reading.zoom,
        value
          .clone()
          .try_into::<f64>()
          .map_err(|_| "expected number")
          .and_then(Zoom::try_from),
      ),
      "reading.inactive_brightness" => replace_setting(
        &mut self.reading.inactive_brightness,
        &previous.reading.inactive_brightness,
        value
          .clone()
          .try_into::<f64>()
          .map_err(|_| "expected number")
          .and_then(Brightness::try_from),
      ),
      "ocr.language" => replace_setting(
        &mut self.ocr.language,
        &previous.ocr.language,
        value
          .as_str()
          .ok_or("expected string")
          .and_then(|value| Language::try_from(value.to_owned())),
      ),
      _ => Err("unknown setting"),
    }
    .map_err(str::to_owned)
  }

  pub fn ascii_glyphs(&self) -> bool {
    self.appearance.ascii_glyphs
  }

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

  #[cfg(test)]
  fn example() -> crate::Result<String> {
    Ok(toml::to_string_pretty(&Self::default())?)
  }
}

#[derive(Debug, Default)]
pub struct Loaded {
  pub source: Source,
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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Source {
  #[default]
  Disabled,
  File {
    path: PathBuf,
    explicit: bool,
  },
}

impl Source {
  fn file(path: &Path, explicit: bool) -> Self {
    let path = if path.is_absolute() {
      path.to_owned()
    } else {
      std::env::current_dir()
        .map_or_else(|_| path.to_owned(), |cwd| cwd.join(path))
    };
    Self::File { path, explicit }
  }

  pub fn reload(&self, previous: &Settings) -> Loaded {
    let Self::File { path, explicit } = self else {
      let mut loaded = Loaded::default();
      loaded.settings.bindings.remember(&previous.bindings);
      loaded.settings.reference.generation = previous.reference.generation + 1;
      return loaded;
    };
    let mut loaded = match std::fs::read_to_string(path) {
      Ok(text) => parse_with(path, &text, previous),
      Err(error) if !explicit && error.kind() == io::ErrorKind::NotFound => {
        let mut loaded = Loaded::default();
        loaded.settings.bindings.remember(&previous.bindings);
        loaded
      }
      Err(error) => finish(path, previous.clone(), &[error.to_string()]),
    };
    loaded.source = self.clone();
    loaded.settings.reference.generation = previous.reference.generation + 1;
    loaded
  }
}

#[cfg(test)]
fn load(path: &Path, explicit: bool) -> Loaded {
  Source::file(path, explicit).reload(&Settings::default())
}

#[cfg(test)]
fn parse(path: &Path, text: &str) -> Loaded {
  parse_with(path, text, &Settings::default())
}

fn parse_with(path: &Path, text: &str, previous: &Settings) -> Loaded {
  let value = match toml::from_str::<toml::Value>(text) {
    Ok(value) => value,
    Err(error) => {
      return finish(path, previous.clone(), &[error.to_string()]);
    }
  };
  let mut settings = Settings::default();
  settings.reference.generation = previous.reference.generation;
  let mut warnings = Vec::new();
  if let Some(root) = value.as_table() {
    for (section, value) in root {
      if section == "keybindings" {
        settings.bindings =
          bindings::parse(value, &previous.bindings, path, &mut warnings);
        continue;
      }
      if !matches!(section.as_str(), "appearance" | "reading" | "ocr") {
        warnings.push(format!("unknown setting {section}"));
        continue;
      }
      let Some(table) = value.as_table() else {
        warnings.push(format!("{section}: expected table"));
        match section.as_str() {
          "appearance" => settings.appearance = previous.appearance.clone(),
          "reading" => settings.reading = previous.reading.clone(),
          "ocr" => settings.ocr = previous.ocr.clone(),
          _ => {}
        }
        settings.reference.sources.extend(
          previous
            .reference
            .sources
            .iter()
            .filter(|(name, _)| name.starts_with(&format!("{section}.")))
            .map(|(name, source)| (name.clone(), source.clone())),
        );
        continue;
      };
      for (key, value) in table {
        let name = format!("{section}.{key}");
        let result = settings.apply(&name, value, previous, path);
        if let Err(error) = result {
          warnings.push(format!("{name}: {error}"));
          let source = previous
            .reference
            .sources
            .get(&name)
            .filter(|_| name != "appearance.theme")
            .cloned()
            .unwrap_or_default();
          settings.reference.sources.insert(name, source);
        } else {
          settings
            .reference
            .sources
            .insert(name, vpdf_ui::reference::Source::File(path.to_owned()));
        }
      }
    }
  }
  settings.bindings.remember(&previous.bindings);
  finish(path, settings, &warnings)
}

fn replace_setting<T: Clone>(
  target: &mut T,
  previous: &T,
  candidate: Result<T, &'static str>,
) -> Result<(), &'static str> {
  *target = candidate.as_ref().unwrap_or(previous).clone();
  candidate.map(|_| ())
}

fn finish(path: &Path, mut settings: Settings, warnings: &[String]) -> Loaded {
  settings.reference.generation += 1;
  settings.reference.warnings = warnings
    .iter()
    .map(|warning| vpdf_model::text::printable(warning).collect())
    .collect();
  let warning = (!warnings.is_empty()).then(|| {
    let message = format!("Config {}: {}", path.display(), warnings.join("; "));
    vpdf_model::text::printable(&message).collect()
  });
  Loaded { settings, warning, source: Source::Disabled }
}

#[cfg(test)]
mod tests;
