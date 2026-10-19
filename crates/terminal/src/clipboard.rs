use crate::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::io::Write;

/// OSC 52 with base64 `data`. Under tmux both plainly, which tmux takes
/// with `set-clipboard on`, and wrapped, which passes through to the
/// terminal with `allow-passthrough` ([tmux input.c][tmux]).
///
/// [tmux]: https://github.com/tmux/tmux/blob/3.8/input.c#L3379-L3380
pub fn osc52(out: &mut impl Write, data: &str, tmux: bool) -> Result<()> {
  write!(out, "\x1b]52;c;{data}\x07")?;
  if tmux {
    write!(out, "\x1bPtmux;\x1b\x1b]52;c;{data}\x07\x1b\\")?;
  }
  Ok(())
}

pub fn image_remote(
  out: &mut impl Write,
  png: &[u8],
  tmux: bool,
) -> Result<()> {
  let packet = |out: &mut _, body: &str| {
    let bytes = format!("\x1b]5522;{body}\x1b\\");
    crate::terminal::passthrough(out, bytes.as_bytes(), tmux)
  };
  packet(out, "type=write")?;
  for chunk in png.chunks(4095) {
    packet(
      out,
      &format!("type=wdata:mime=aW1hZ2UvcG5n;{}", STANDARD.encode(chunk)),
    )?;
  }
  packet(out, "type=wdata")
}

pub fn image_status(body: &str) -> Option<Result<()>> {
  let fields: Vec<_> = body.split(':').collect();
  if !fields.contains(&"type=write") {
    return None;
  }
  let status = fields.iter().find_map(|field| field.strip_prefix("status="))?;
  Some(if status == "DONE" {
    Ok(())
  } else {
    Err(anyhow::anyhow!("Image clipboard failed: {status}"))
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn tmux_gets_plain_and_wrapped_osc52() -> Result<()> {
    let plain: &[u8] = b"\x1b]52;c;aGk=\x07";
    let wrapped: &[u8] = b"\x1bPtmux;\x1b\x1b]52;c;aGk=\x07\x1b\\";
    for (tmux, want) in
      [(false, plain.to_vec()), (true, [plain, wrapped].concat())]
    {
      let mut out = Vec::new();
      osc52(&mut out, "aGk=", tmux)?;
      assert_eq!(out, want, "tmux {tmux}");
    }
    Ok(())
  }
  #[test]
  fn image_packets_roundtrip() -> Result<()> {
    use crate::decode::{Event, Parser, Reply};
    use std::collections::VecDeque;
    for size in [1, 4095, 4096, 8191] {
      let png: Vec<u8> = (0..size).map(|n| n as u8).collect();
      let mut out = Vec::new();
      image_remote(&mut out, &png, false)?;
      let mut events = VecDeque::new();
      Parser::default().feed(&out, &mut events);
      assert_eq!(
        events.pop_front(),
        Some(Event::Reply(Reply::Osc {
          code: 5522,
          body: "type=write".into()
        }))
      );
      assert_eq!(
        events.pop_back(),
        Some(Event::Reply(Reply::Osc {
          code: 5522,
          body: "type=wdata".into()
        }))
      );
      let mut decoded = Vec::new();
      for event in events {
        let Event::Reply(Reply::Osc { code: 5522, body }) = event else {
          panic!("Unexpected packet")
        };
        let payload =
          body.strip_prefix("type=wdata:mime=aW1hZ2UvcG5n;").unwrap();
        let chunk = STANDARD.decode(payload)?;
        assert!(chunk.len() <= 4096);
        decoded.extend(chunk);
      }
      assert_eq!(decoded, png);
    }
    let mut wrapped = Vec::new();
    image_remote(&mut wrapped, b"hi", true)?;
    let expected: [&[u8]; 3] = [
      b"\x1bPtmux;\x1b\x1b]5522;type=write\x1b\x1b\\\x1b\\",
      b"\x1bPtmux;\x1b\x1b]5522;type=wdata:mime=aW1hZ2UvcG5n;\
        aGk=\x1b\x1b\\\x1b\\",
      b"\x1bPtmux;\x1b\x1b]5522;type=wdata\x1b\x1b\\\x1b\\",
    ];
    assert_eq!(wrapped, expected.concat());
    assert!(image_status("type=write:status=DONE").unwrap().is_ok());
    assert!(image_status("type=write:status=EPERM").unwrap().is_err());
    assert!(image_status("type=read:status=DONE").is_none());
    Ok(())
  }
}
