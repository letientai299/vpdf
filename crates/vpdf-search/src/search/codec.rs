//! A page index as bytes for the disk cache, without its page number,
//! so a record can serve whichever page holds the same text.
use super::{Cover, Index, Join, Run};
use crate::Result;
use crate::text::PageId;
use anyhow::{Context, bail, ensure};

impl Index {
  /// Little-endian: text, runs, then printed lines, each counted.
  pub fn encode(&self, out: &mut Vec<u8>) {
    put_bytes(out, self.text.as_bytes());
    put(out, self.runs.len());
    for run in &self.runs {
      for field in [run.byte, run.line, run.glyph] {
        out.extend_from_slice(&field.to_le_bytes());
      }
      out.push(u8::from(run.stepped));
    }
    put(out, self.lines.len());
    for line in &self.lines {
      put_bytes(out, line.as_bytes());
    }
    for join in &self.joins {
      out.push(match join {
        Join::Space => 0,
        Join::Continue => 1,
        Join::Hyphen => 2,
      });
    }
    put(out, self.covers.len());
    for cover in &self.covers {
      for value in [cover.start, cover.end, cover.line, cover.glyph] {
        out.extend_from_slice(&value.to_le_bytes());
      }
    }
  }

  /// Rejects any record `encode` could not have written, so a damaged
  /// cache file cannot make a lookup panic.
  pub fn decode(page: PageId, bytes: &[u8]) -> Result<Self> {
    let mut reader = Reader(bytes);
    let text = String::from_utf8(reader.bytes()?.to_vec())?;
    let count = reader.count(13)?;
    let mut runs = Vec::with_capacity(count);
    for _ in 0..count {
      let (byte, line, glyph) = (reader.u32()?, reader.u32()?, reader.u32()?);
      let stepped = match reader.take(1)?[0] {
        0 => false,
        1 => true,
        _ => bail!("Invalid search record run"),
      };
      runs.push(Run { byte, line, glyph, stepped });
    }
    let count = reader.count(4)?;
    let mut lines: Vec<Box<str>> = Vec::with_capacity(count);
    for _ in 0..count {
      lines.push(std::str::from_utf8(reader.bytes()?)?.into());
    }
    let glyphs: Vec<_> =
      lines.iter().map(|line| line.chars().count()).collect();
    let joins = (0..lines.len())
      .map(|_| match reader.take(1)?[0] {
        0 => Ok(Join::Space),
        1 => Ok(Join::Continue),
        2 => Ok(Join::Hyphen),
        _ => bail!("Invalid search line join"),
      })
      .collect::<Result<Vec<_>>>()?;
    let count = reader.count(16)?;
    let mut covers: Vec<Cover> = Vec::with_capacity(count);
    for _ in 0..count {
      let cover = Cover {
        start: reader.u32()?,
        end: reader.u32()?,
        line: reader.u32()?,
        glyph: reader.u32()?,
      };
      ensure!(
        cover.start < cover.end && cover.end as usize <= text.len(),
        "Invalid search source coverage"
      );
      ensure!((cover.line as usize) < glyphs.len(), "Invalid coverage line");
      ensure!(
        covers.last().is_none_or(|last| last.end <= cover.start),
        "Overlapping search source coverage"
      );
      ensure!(
        text.is_char_boundary(cover.start as usize)
          && text.is_char_boundary(cover.end as usize),
        "Coverage splits UTF-8 character"
      );
      ensure!(
        (cover.glyph as usize) < glyphs[cover.line as usize],
        "Coverage past glyphs"
      );
      covers.push(cover);
    }
    validate(&text, &runs, &glyphs)?;
    ensure!(reader.0.is_empty(), "Trailing bytes in search record");
    Ok(Self { page, text, runs, lines, joins, covers, ..Self::default() })
  }
}

/// Runs start at byte 0, rise strictly inside the text, name one of
/// `lines`, and their stepped glyphs stay within `u32`.
fn validate(text: &str, runs: &[Run], glyphs: &[usize]) -> Result<()> {
  let len = u32::try_from(text.len())?;
  ensure!(
    runs.first().map(|run| run.byte) == (len > 0).then_some(0),
    "Search record runs miss the text start"
  );
  for (at, run) in runs.iter().enumerate() {
    let end = runs.get(at + 1).map_or(len, |next| next.byte);
    ensure!(run.byte < end, "Search record runs out of order");
    ensure!(
      (run.line as usize) < glyphs.len(),
      "Search record run past its lines"
    );
    let last = if run.stepped {
      run.glyph.checked_add(end - run.byte - 1).context("Run past u32")?
    } else {
      run.glyph
    };
    ensure!((last as usize) < glyphs[run.line as usize], "Run past glyphs");
  }
  Ok(())
}

fn put(out: &mut Vec<u8>, count: usize) {
  out.extend_from_slice(&(count as u32).to_le_bytes());
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
  put(out, bytes.len());
  out.extend_from_slice(bytes);
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
  fn take(&mut self, len: usize) -> Result<&'a [u8]> {
    ensure!(len <= self.0.len(), "Truncated search record");
    let (head, rest) = self.0.split_at(len);
    self.0 = rest;
    Ok(head)
  }

  fn u32(&mut self) -> Result<u32> {
    let bytes = self.take(4)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
  }

  /// A count of items at least `size` bytes each, bounded by what is
  /// left, so a damaged count cannot reserve gigabytes.
  fn count(&mut self, size: usize) -> Result<usize> {
    let count = self.u32()? as usize;
    ensure!(
      count.saturating_mul(size) <= self.0.len(),
      "Truncated search record"
    );
    Ok(count)
  }

  fn bytes(&mut self) -> Result<&'a [u8]> {
    let len = self.u32()? as usize;
    self.take(len)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::search::compile;
  use crate::search::tests::sample;

  fn encoded(index: &Index) -> Vec<u8> {
    let mut out = Vec::new();
    index.encode(&mut out);
    out
  }

  /// Decoded records search, excerpt, and encode as their source did.
  #[test]
  fn records_round_trip() -> Result<()> {
    let pages: [&[&str]; 3] =
      [&["Straße ﬁsh inter-", "national  café", "日本語 x\ty"], &["-"], &[]];
    for lines in pages {
      let index = Index::new(PageId(3), &sample(lines));
      let bytes = encoded(&index);
      let decoded = Index::decode(PageId(7), &bytes)?;
      assert_eq!(encoded(&decoded), bytes, "{lines:?}");
      for query in ["international", "café", "日本", "x y", "Straße", "STRASSE"]
      {
        let pattern = compile(query, false)?;
        let hits = |index: &Index| -> Result<Vec<_>> {
          Ok(
            index
              .matches(&pattern, 100)?
              .into_iter()
              .map(|hit| (hit.first.line, hit.first.glyph, hit.excerpt))
              .collect(),
          )
        };
        assert_eq!(hits(&decoded)?, hits(&index)?, "{query}");
      }
      assert!(
        decoded
          .matches(&compile("x", false)?, 1)?
          .iter()
          .all(|hit| { hit.first.page == PageId(7) })
      );
    }
    Ok(())
  }

  /// Every truncation and damaged field fails to decode, without panics.
  #[test]
  fn damaged_records_fail() {
    let index = Index::new(PageId(0), &sample(&["ab日c", "next line"]));
    let bytes = encoded(&index);
    for len in 0..bytes.len() {
      assert!(Index::decode(PageId(0), &bytes[..len]).is_err(), "{len}");
    }
    let text_len = 4 + "ab日c next line ".len();
    let damage: [(usize, u8); 5] = [
      // Invalid UTF-8 inside the text.
      (6, 0xff),
      // First run off byte 0.
      (text_len + 4, 1),
      // Second run before the first.
      (text_len + 4 + 13, 0),
      // Stepped flag neither 0 nor 1.
      (text_len + 4 + 12, 2),
      // Huge run count.
      (text_len + 3, 0x7f),
    ];
    for (at, value) in damage {
      let mut broken = bytes.clone();
      broken[at] = value;
      assert!(Index::decode(PageId(0), &broken).is_err(), "{at}");
    }
    let mut longer = bytes;
    longer.push(0);
    assert!(Index::decode(PageId(0), &longer).is_err(), "trailing");
  }
}
