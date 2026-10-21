use super::{Step, pair};
use crate::Result;
use vpdf_model::geometry::Screen;

pub struct Source {
  name: String,
  sha256: String,
}

impl Source {
  pub fn name(&self) -> &str {
    &self.name
  }
  pub fn sha256(&self) -> &str {
    &self.sha256
  }
}

pub struct Scenario {
  sources: Vec<Source>,
  window: (u32, u32),
  cell: (u16, u16),
  crop: bool,
  steps: Vec<Step>,
}

impl Scenario {
  pub fn parse(text: &str) -> Result<Self> {
    let mut sources = Vec::new();
    let (mut window, mut crop) = (None, None);
    let mut cell = (10, 20);
    let mut steps = Vec::new();
    for (number, line) in text.lines().enumerate() {
      if let Some(header) = line.trim().strip_prefix('#') {
        let (name, value) = header
          .trim()
          .split_once(char::is_whitespace)
          .unwrap_or((header.trim(), ""));
        match name {
          "pdf" => sources.push(source(value.trim())?),
          "window" => {
            anyhow::ensure!(window.is_none(), "Repeated scenario window");
            let (width, height) = pair::<u32>(value.trim())?;
            anyhow::ensure!(
              width > 0 && height > 0 && width <= 8192 && height <= 8192,
              "Invalid scenario window"
            );
            window = Some((width, height));
          }
          "cell" => {
            cell = pair(value.trim())?;
            anyhow::ensure!(cell.0 > 0 && cell.1 > 0, "Invalid scenario cell");
          }
          "crop" => {
            anyhow::ensure!(crop.is_none(), "Repeated scenario crop mode");
            crop = Some(match value.trim() {
              "on" => true,
              "off" => false,
              _ => anyhow::bail!("Invalid scenario crop mode"),
            });
          }
          _ => {}
        }
        continue;
      }
      let step = Step::parse(line).map_err(|error| {
        anyhow::anyhow!("Scenario line {}: {error}", number + 1)
      })?;
      if let Some(step) = step {
        steps.push(step);
      }
    }
    anyhow::ensure!(!sources.is_empty(), "Scenario PDF header missing");
    anyhow::ensure!(!steps.is_empty(), "Scenario steps missing");
    let scenario = Self {
      sources,
      window: window
        .ok_or_else(|| anyhow::anyhow!("Scenario window header missing"))?,
      cell,
      crop: crop
        .ok_or_else(|| anyhow::anyhow!("Scenario crop header missing"))?,
      steps,
    };
    anyhow::ensure!(
      scenario.screen().columns > 0 && scenario.screen().rows >= 2,
      "Scenario grid is too small"
    );
    Ok(scenario)
  }

  pub fn sources(&self) -> &[Source] {
    &self.sources
  }
  pub fn window(&self) -> (u32, u32) {
    self.window
  }
  pub fn crop(&self) -> bool {
    self.crop
  }
  pub fn steps(&self) -> &[Step] {
    &self.steps
  }
  pub fn screen(&self) -> Screen {
    Screen {
      columns: (self.window.0 / u32::from(self.cell.0)) as u16,
      rows: (self.window.1 / u32::from(self.cell.1)) as u16,
      cell_width: self.cell.0,
      cell_height: self.cell.1,
      origin: (0, 0),
    }
  }
}

fn source(text: &str) -> Result<Source> {
  let (name, sha256) = text
    .split_once(char::is_whitespace)
    .ok_or_else(|| anyhow::anyhow!("Scenario PDF needs name and hash"))?;
  let sha256 = sha256.trim();
  anyhow::ensure!(
    !name.is_empty()
      && name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'),
    "Invalid scenario PDF name"
  );
  anyhow::ensure!(
    sha256.len() == 64 && sha256.bytes().all(|ch| ch.is_ascii_hexdigit()),
    "Invalid scenario PDF hash"
  );
  Ok(Source { name: name.into(), sha256: sha256.to_ascii_lowercase() })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn headers_are_required() -> Result<()> {
    let text = format!(
      "# pdf paper {}\n# window 1000 800\n# crop off\nwait complete\nquit\n",
      "a".repeat(64)
    );
    let scenario = Scenario::parse(&text)?;
    assert_eq!(scenario.screen().columns, 100);
    assert_eq!(scenario.screen().rows, 40);
    assert_eq!(scenario.sources()[0].name(), "paper");
    for invalid in [
      text.replace("# crop off\n", ""),
      text.replace("# window 1000 800\n", ""),
      text.replace("paper", "../paper"),
      text.replace(&"a".repeat(64), "bad"),
      text.replace("1000 800", "1 1"),
      text.replace("off", "maybe"),
    ] {
      assert!(Scenario::parse(&invalid).is_err());
    }
    Ok(())
  }
}
