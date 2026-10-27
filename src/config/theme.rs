use serde::{Serialize, Serializer};
use std::path::{Path, PathBuf};
use vpdf_model::theme::{Colors, Palette, Theme};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ThemeChoice {
  Builtin(Theme),
  File { path: PathBuf, palette: Palette },
}

impl Default for ThemeChoice {
  fn default() -> Self {
    Self::Builtin(Theme::System)
  }
}

impl From<Theme> for ThemeChoice {
  fn from(theme: Theme) -> Self {
    Self::Builtin(theme)
  }
}

impl Serialize for ThemeChoice {
  fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
    match self {
      Self::Builtin(theme) => serializer.serialize_str(theme.name()),
      Self::File { path, .. } => {
        serializer.serialize_str(&path.to_string_lossy())
      }
    }
  }
}

impl ThemeChoice {
  pub(crate) fn is_path(value: &str) -> bool {
    value.contains('/')
      || value
        .rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("toml"))
  }

  pub(crate) fn load(value: &str, base: &Path) -> Result<Self, String> {
    if !Self::is_path(value) {
      return Theme::parse(value)
        .map(Self::Builtin)
        .ok_or_else(|| Theme::EXPECTED.to_owned());
    }
    let expanded = crate::cmdline::expand(value);
    let path = std::path::absolute(base.join(expanded))
      .map_err(|error| error.to_string())?;
    let text = std::fs::read_to_string(&path)
      .map_err(|error| format!("{}: {error}", path.display()))?;
    let palette = Palette::parse(&text)
      .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(Self::File { path, palette })
  }

  pub(super) fn colors(&self, system: Colors) -> Colors {
    match self {
      Self::Builtin(Theme::System) => system,
      Self::Builtin(theme) => Colors::Palette(theme.palette()),
      Self::File { palette, .. } => Colors::Palette(*palette),
    }
  }
}
