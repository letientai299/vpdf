use crate::Result;
use mupdf::pdf::{PdfDocument, PdfObject};
use mupdf::{Buffer, Font};
use std::fmt::Write;
use vpdf_model::theme::Palette;

const FONT: &[u8] = include_bytes!("../../assets/DejaVuSansMono.ttf");
const CELL: f32 = 6.6226;
const COLUMNS: usize = 77;
const LINE_HEIGHT: f32 = 18.0;

pub(super) struct Writer {
  pub native: PdfDocument,
  font: Font,
  palette: Palette,
  resources: PdfObject,
  content: String,
  links: Vec<(f32, Link)>,
  y: f32,
}

enum Link {
  Resolved(PdfObject),
  Named(String),
}

impl Writer {
  pub fn new(mut native: PdfDocument, palette: Palette) -> Result<Self> {
    let font = Font::from_bytes("DejaVu Sans Mono", FONT)?;
    let mut fonts = native.new_dict()?;
    fonts.dict_put("F1", native.add_font(&font)?)?;
    let mut resources = native.new_dict()?;
    resources.dict_put("Font", fonts)?;
    Ok(Self {
      native,
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
      for (y, dest) in self.links.drain(..) {
        let mut annot = self.native.new_object_from_str(&format!(
          "<< /Type /Annot /Subtype /Link /Rect [48 {} 564 {}] \
           /Border [0 0 1] >>",
          y - 2.0,
          y + 12.0
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
    for line in wrap(&self.encode(&format!("== {text} =="))?, 60) {
      self.position()?;
      self.draw(&line, 48.0, self.palette.accent, 14.0)?;
      self.y -= 26.0;
    }
    Ok(())
  }

  pub fn section(&mut self) -> Result<(i32, f32)> {
    if self.y < 10.0 * LINE_HEIGHT {
      self.flush()?;
    }
    let (page, y) = self.position()?;
    Ok((page, (y - 4.0).max(0.0)))
  }

  pub fn link(&mut self, target: PdfObject) {
    self.links.push((self.y + LINE_HEIGHT, Link::Resolved(target)));
  }

  pub fn named_link(&mut self, target: String) {
    self.links.push((self.y + LINE_HEIGHT, Link::Named(target)));
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
    let color = if description.starts_with("Command:") {
      self.palette.highlight
    } else {
      self.palette.foreground
    };
    if keys.starts_with(':') {
      let mut runs = Vec::new();
      for word in keys.split_inclusive(' ') {
        let color = if word.starts_with(':') || word.trim().is_empty() {
          self.palette.highlight
        } else {
          self.palette.r#match
        };
        runs.push((word, color));
      }
      self.colored_at(&runs, 0)?;
      return self.description(description);
    }
    self.styled(keys, self.palette.r#match)?;
    self.styled_at(description, color, 24)
  }

  pub fn description(&mut self, text: &str) -> Result<()> {
    let runs = inline(text, self.palette);
    self.colored_at(&runs, 24)
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
    let mut text = String::new();
    let mut colors = Vec::new();
    for &(run, color) in runs {
      let encoded = self.encode(run)?;
      colors.extend(encoded.chars().map(|_| color));
      text.push_str(&encoded);
    }
    let all_chars: Vec<_> = text.chars().collect();
    let mut offset = 0;
    for line in wrap(&text, COLUMNS - indent) {
      self.position()?;
      let chars: Vec<_> = line.chars().collect();
      let mut start = 0;
      while start < chars.len() {
        let color = colors[offset + start];
        let end = (start + 1..chars.len())
          .find(|&index| colors[offset + index] != color)
          .unwrap_or(chars.len());
        let run: String = chars[start..end].iter().collect();
        self.draw(&run, 48.0 + (indent + start) as f32 * CELL, color, 11.0)?;
        start = end;
      }
      offset += chars.len();
      while all_chars.get(offset).is_some_and(|ch| ch.is_whitespace()) {
        offset += 1;
      }
      self.y -= LINE_HEIGHT;
    }
    Ok(())
  }

  pub fn position(&mut self) -> Result<(i32, f32)> {
    if self.y < 48.0 {
      self.flush()?;
    }
    Ok((self.native.page_count()?, 780.0 - self.y))
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
    let split =
      remaining[..end].rfind(' ').filter(|&index| index > 0).unwrap_or(end);
    lines.push(remaining[..split].into());
    remaining = remaining[split..].trim_start();
  }
  lines.push(remaining.into());
  lines
}

fn inline(text: &str, palette: Palette) -> Vec<(&str, [u8; 3])> {
  if text.starts_with("See |") && text.ends_with("|.") {
    return vec![
      (&text[..4], palette.foreground),
      (&text[4..text.len() - 1], palette.link),
      (&text[text.len() - 1..], palette.foreground),
    ];
  }
  let mut runs = Vec::new();
  let mut rest = text;
  while let Some(start) = rest.find('`') {
    let Some(end) = rest[start + 1..].find('`') else { break };
    runs.push((&rest[..start], palette.foreground));
    runs.push((&rest[start + 1..start + 1 + end], palette.r#match));
    rest = &rest[start + end + 2..];
  }
  runs.push((rest, palette.foreground));
  runs
}
