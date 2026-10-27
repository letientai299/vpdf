use crate::Result;
use mupdf::pdf::{PdfDocument, PdfObject};
use mupdf::{Buffer, Font};
use std::fmt::Write;
use vpdf_model::theme::Palette;

const FONT: &[u8] = include_bytes!("../../../ui/assets/VpdfMonoText.ttf");
#[derive(serde::Deserialize)]
struct Typography {
  body: f32,
  leading: f32,
  paragraph: f32,
}

impl Typography {
  fn line_height(&self) -> f32 {
    self.body + self.leading
  }
  fn cell(&self) -> f32 {
    self.body * 0.5
  }
  fn columns(&self) -> usize {
    (516.0 / self.cell()) as usize
  }
}

pub(super) struct Writer {
  pub native: PdfDocument,
  typography: Typography,
  font: Font,
  palette: Palette,
  resources: PdfObject,
  content: String,
  links: Vec<([f32; 4], Link)>,
  y: f32,
}

enum Link {
  Resolved(PdfObject),
  Named(String),
}

impl Writer {
  pub fn new(mut native: PdfDocument, palette: Palette) -> Result<Self> {
    let font = Font::from_bytes("Vpdf Mono", FONT)?;
    let mut fonts = native.new_dict()?;
    fonts.dict_put("F1", native.add_font(&font)?)?;
    let mut resources = native.new_dict()?;
    resources.dict_put("Font", fonts)?;
    Ok(Self {
      native,
      typography: serde_json::from_str(include_str!(
        "../../../../scripts/help/typography.json"
      ))?,
      font,
      palette,
      resources,
      content: String::new(),
      links: Vec::new(),
      y: 744.0,
    })
  }

  pub fn flush(&mut self) -> Result<()> {
    if self.content.is_empty() {
      return Ok(());
    }
    let page = self.native.new_page((612, 792))?;
    let background = format!(
      "{} rg 0 0 612 792 re f\n",
      super::style::rgb(self.palette.background)
    );
    let stream = self.native.add_stream(
      &Buffer::from_copied_bytes(
        format!("{background}{}", self.content).as_bytes(),
      )?,
      None,
      false,
    )?;
    let mut object = page.object();
    object.dict_put("Resources", self.resources.clone())?;
    object.dict_put("Contents", stream)?;
    if !self.links.is_empty() {
      let mut annots = self.native.new_array()?;
      for (rect, dest) in self.links.drain(..) {
        let mut annot = self.native.new_object_from_str(&format!(
          "<< /Type /Annot /Subtype /Link /Rect [{} {} {} {}] \
           /Border [0 0 0] >>",
          rect[0], rect[1], rect[2], rect[3]
        ))?;
        match dest {
          Link::Resolved(dest) => annot.dict_put("Dest", dest)?,
          Link::Named(name) => {
            let mut action = self.native.new_dict()?;
            action.dict_put("S", self.native.new_name("URI")?)?;
            action.dict_put(
              "URI",
              self.native.new_string(&format!("vpdf-help:{name}"))?,
            )?;
            annot.dict_put("A", action)?;
          }
        }
        annots.array_push(self.native.add_object(&annot)?)?;
      }
      object.dict_put("Annots", annots)?;
    }
    self.content.clear();
    self.y = 744.0;
    Ok(())
  }

  pub fn heading(&mut self, text: &str) -> Result<()> {
    for line in wrap(
      &self.encode(text)?,
      (self.typography.columns() as f32 / 1.4) as usize,
    ) {
      self.position()?;
      self.draw(
        &line,
        48.0,
        self.palette.accent,
        self.typography.body * 1.4,
      )?;
      self.y -= 26.0;
    }
    Ok(())
  }

  pub fn section(&mut self, lines: u8) -> Result<(i32, f32)> {
    if self.y < 48.0 + f32::from(lines - 1) * self.typography.line_height() {
      self.flush()?;
    }
    let (page, y) = self.position()?;
    Ok((page, y))
  }

  pub fn link(&mut self, target: PdfObject) {
    let y = self.y + self.typography.line_height();
    self.links.push((
      [48.0, y - 2.0, 564.0, y + self.typography.body],
      Link::Resolved(target),
    ));
  }

  pub fn named_link(&mut self, target: String) {
    let y = self.y + self.typography.line_height();
    self.links.push((
      [48.0, y - 2.0, 564.0, y + self.typography.body],
      Link::Named(target),
    ));
  }

  pub fn reference_line(
    &mut self,
    line: &super::Line,
    indent: bool,
  ) -> Result<()> {
    match line {
      super::Line::Text(text) if indent => self.description(text),
      super::Line::Text(text) => self.line(text),
      super::Line::Binding { keys, description } => {
        self.binding(keys, description)
      }
    }
  }

  pub fn binding(&mut self, keys: &str, description: &str) -> Result<()> {
    self.entry(keys, description, &[])
  }

  pub fn entry(
    &mut self,
    keys: &str,
    description: &str,
    targets: &[String],
  ) -> Result<()> {
    let labels: Vec<_> =
      targets.iter().map(|target| format!(" :help {target}")).collect();
    let mut runs = Vec::new();
    if keys.starts_with(':') {
      for word in keys.split_inclusive(' ') {
        let color = if word.starts_with(':') || word.trim().is_empty() {
          self.palette.highlight
        } else {
          self.palette.r#match
        };
        runs.push((word, color));
      }
    } else {
      runs.push((keys, self.palette.r#match));
    }
    runs.push(("  ", self.palette.foreground));
    runs.extend(inline(description, self.palette));
    runs.extend(labels.iter().map(|label| (label.as_str(), self.palette.link)));
    self.paragraph(&runs, 0, targets)
  }

  pub fn description(&mut self, text: &str) -> Result<()> {
    let runs = inline(text, self.palette);
    self.colored_at(&runs, 2)
  }

  pub fn line(&mut self, text: &str) -> Result<()> {
    let color = if ["Read:", "Dispatch:", "Back:"]
      .iter()
      .any(|prefix| text.starts_with(prefix))
    {
      self.palette.link
    } else if text.starts_with("Removed default:")
      || text.starts_with("Rejected")
      || text.starts_with("Warning:")
    {
      self.palette.error
    } else if text.starts_with("Action:") {
      self.palette.highlight
    } else if text.ends_with(':') {
      self.palette.accent
    } else {
      self.palette.foreground
    };
    self.styled(text, color)
  }

  fn encode(&self, text: &str) -> Result<String> {
    let mut encoded = String::new();
    for ch in text.chars() {
      if ch.is_control() || self.font.encode_character(ch as i32)? == 0 {
        encoded.push_str(&ch.escape_unicode().to_string());
      } else {
        if ch == '\\' {
          encoded.push('\\');
        }
        encoded.push(ch);
      }
    }
    Ok(encoded)
  }

  fn styled(&mut self, text: &str, color: [u8; 3]) -> Result<()> {
    self.styled_at(text, color, 0)
  }

  fn styled_at(
    &mut self,
    text: &str,
    color: [u8; 3],
    indent: usize,
  ) -> Result<()> {
    self.colored_at(&[(text, color)], indent)
  }

  fn colored_at(
    &mut self,
    runs: &[(&str, [u8; 3])],
    indent: usize,
  ) -> Result<()> {
    self.paragraph(runs, indent, &[])
  }

  fn paragraph(
    &mut self,
    runs: &[(&str, [u8; 3])],
    indent: usize,
    targets: &[String],
  ) -> Result<()> {
    let mut text = String::new();
    let mut colors = Vec::new();
    for &(run, color) in runs {
      let encoded = self.encode(run)?;
      colors.extend(encoded.chars().map(|_| color));
      text.push_str(&encoded);
    }
    if text.is_empty() {
      self.y -= self.typography.paragraph;
      return Ok(());
    }
    let all_chars: Vec<_> = text.chars().collect();
    let mut at = all_chars.len();
    let mut tags = Vec::new();
    for target in targets.iter().rev() {
      let len = self.encode(&format!(":help {target}"))?.chars().count();
      at -= len;
      tags.push((at, len, target));
      at -= 1;
    }
    tags.reverse();
    let cell = self.typography.cell();
    let columns = self.typography.columns();
    let mut offset = 0;
    let lines = wrap(&text, columns - indent);
    for (index, line) in lines.iter().enumerate() {
      self.position()?;
      let chars: Vec<_> = line.chars().collect();
      let spaces = chars.iter().filter(|&&ch| ch == ' ').count();
      let spacing = if index + 1 < lines.len() && spaces > 0 {
        (columns - indent - chars.len()) as f32 * cell / spaces as f32
      } else {
        0.0
      };
      let mut extra = 0.0;
      let mut start = 0;
      while start < chars.len() {
        let color = colors[offset + start];
        let end = (start + 1..chars.len())
          .find(|&index| {
            colors[offset + index] != color
              || (spacing > 0.0 && chars[index - 1] == ' ')
          })
          .unwrap_or(chars.len());
        let run: String = chars[start..end].iter().collect();
        self.draw(
          &run,
          48.0 + (indent + start) as f32 * cell + extra,
          color,
          self.typography.body,
        )?;
        extra += run.chars().filter(|&ch| ch == ' ').count() as f32 * spacing;
        start = end;
      }
      let advance = |index: usize| {
        index as f32 * cell
          + chars[..index].iter().filter(|&&ch| ch == ' ').count() as f32
            * spacing
      };
      for &(at, len, target) in &tags {
        let first = at.max(offset);
        let last = (at + len).min(offset + chars.len());
        if first < last {
          let x = 48.0 + indent as f32 * cell;
          self.links.push((
            [
              x + advance(first - offset),
              self.y - 2.0,
              x + advance(last - offset),
              self.y + self.typography.body,
            ],
            Link::Named(target.clone()),
          ));
        }
      }
      offset += chars.len();
      while all_chars.get(offset).is_some_and(|ch| ch.is_whitespace()) {
        offset += 1;
      }
      self.y -= self.typography.line_height();
    }
    Ok(())
  }

  pub fn position(&mut self) -> Result<(i32, f32)> {
    if self.y < 48.0 {
      self.flush()?;
    }
    Ok((self.native.page_count()?, 792.0 - self.y - self.typography.body))
  }

  fn draw(
    &mut self,
    text: &str,
    x: f32,
    color: [u8; 3],
    size: f32,
  ) -> Result<()> {
    let mut glyphs = String::new();
    for ch in text.chars() {
      write!(glyphs, "{:04X}", self.font.encode_character(ch as i32)?)?;
    }
    writeln!(
      self.content,
      "{} rg BT /F1 {size} Tf 1 0 0 1 {x} {} Tm <{glyphs}> Tj ET",
      super::style::rgb(color),
      self.y
    )?;
    Ok(())
  }
}

fn wrap(text: &str, columns: usize) -> Vec<String> {
  let mut lines = Vec::new();
  let mut remaining = text;
  while remaining.chars().count() > columns {
    let end = remaining
      .char_indices()
      .nth(columns)
      .map_or(remaining.len(), |(index, _)| index);
    let mut split =
      remaining[..end].rfind(' ').filter(|&index| index > 0).unwrap_or(end);
    if remaining[..split].ends_with(":help")
      && let Some(before) = remaining[..split - 5].rfind(' ')
      && before > 0
    {
      split = before;
    }
    lines.push(remaining[..split].into());
    remaining = remaining[split..].trim_start();
  }
  lines.push(remaining.into());
  lines
}

fn inline(text: &str, palette: Palette) -> Vec<(&str, [u8; 3])> {
  let mut runs = Vec::new();
  let mut rest = text;
  while let Some(start) = rest.find('`') {
    let Some(end) = rest[start + 1..].find('`') else { break };
    runs.push((&rest[..start], palette.foreground));
    let code = &rest[start + 1..start + 1 + end];
    let color = if code.starts_with(":help ") {
      palette.link
    } else if code.starts_with(':') {
      palette.highlight
    } else {
      palette.r#match
    };
    runs.push((code, color));
    rest = &rest[start + end + 2..];
  }
  runs.push((rest, palette.foreground));
  runs
}
