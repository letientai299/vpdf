use super::overlays::segments;
use super::placement::pieces;
use super::*;

use crate::text::PageId;

fn commands(output: &[u8]) -> Vec<HashMap<String, String>> {
  let text = String::from_utf8_lossy(output);
  let owned = |(key, value): (&str, &str)| (key.to_owned(), value.to_owned());
  let commands = crate::kitty::commands(&text);
  commands.iter().map(|command| command.pairs().map(owned).collect()).collect()
}

#[test]
fn commands_stay_in_session_block() -> Result<()> {
  let mut graphics = Graphics {
    resident: Cache::new(2 * BUDGET.div_ceil((BLOCK - OVERLAYS) as usize)),
    ..Graphics::default()
  };
  let block = graphics.base..=graphics.base + (BLOCK - 1);
  let mut output = Vec::new();
  for page in 0..4 {
    upload(&mut graphics, 0, tile_key(page, (1, 1)), &mut output)?;
  }
  let screen = crate::screen::cells(10, 10);
  let rect = Bounds { x0: 0.0, y0: 0.0, x1: 4.0, y1: 4.0 };
  graphics.highlight(rect, [0; 4], screen, 0, &mut output)?;
  graphics.begin(&mut output)?;
  graphics.release(&mut output)?;
  let commands = commands(&output);
  assert!(commands.iter().any(|c| c.get("d").is_some_and(|d| d == "I")));
  for command in commands {
    if let Some(delete) = command.get("d") {
      assert!(["I", "i"].contains(&delete.as_str()), "{delete}");
    }
    let ids = ["i", "x", "y"]
      .iter()
      .filter(|key| *key == &"i" || command.get("a").is_some_and(|a| a == "d"));
    for key in ids {
      if let Some(id) = command.get(*key) {
        assert!(block.contains(&id.parse()?), "{key}={id}");
      }
    }
  }
  assert_eq!(graphics.free.len(), 1);
  assert_eq!(graphics.next, graphics.base + OVERLAYS + 3);
  Ok(())
}

const SCREEN: Screen = crate::screen::cells(20, 10);

#[test]
fn missing_counts_visible_tiles() -> Result<()> {
  let mut graphics = Graphics::default();
  let key = tile(&mut graphics, 0)?;
  let mut output = Vec::new();
  graphics.begin(&mut output)?;
  assert!(graphics.place_at((0, key), (0.0, 0.0), SCREEN, 0, &mut output)?);
  assert_eq!(graphics.missing_tiles, 0);
  for (origin, expected) in
    [((200.0, 0.0), 0), ((-64.0, 0.0), 0), ((-63.0, 0.0), 1), ((0.0, 0.0), 2)]
  {
    assert!(!graphics.place_at((1, key), origin, SCREEN, 0, &mut output)?);
    assert_eq!(graphics.missing_tiles, expected);
  }
  graphics.end(&mut output)?;
  assert_eq!(graphics.missing_tiles, 2);
  graphics.begin(&mut output)?;
  assert_eq!(graphics.missing_tiles, 0);
  Ok(())
}

/// A resident 64x64 tile of page `page`.
pub(super) fn tile(graphics: &mut Graphics, page: usize) -> Result<TileKey> {
  tile_sized(graphics, page, (64, 64))
}

fn tile_sized(
  graphics: &mut Graphics,
  page: usize,
  size: (i32, i32),
) -> Result<TileKey> {
  let key = tile_key(page, size);
  upload(graphics, 0, key, &mut Vec::new())?;
  Ok(key)
}

fn tile_key(page: usize, (width, height): (i32, i32)) -> TileKey {
  TileKey { page: PageId(page), scale: 1, width, height, ..TileKey::default() }
}

/// Uploads a white `key` tile for `pane`.
fn upload(
  graphics: &mut Graphics,
  pane: u32,
  key: TileKey,
  out: &mut Vec<u8>,
) -> Result<()> {
  let pixels = vec![255; (key.width * key.height * 3) as usize];
  let payload = Payload::Encoded(encode(&pixels)?.into());
  graphics.upload(pane, &Tile { key, payload, render_ms: 0.0 }, out)
}

pub(super) fn has(
  command: &HashMap<String, String>,
  pairs: &[(&str, &str)],
) -> bool {
  pairs
    .iter()
    .all(|(key, value)| command.get(*key).map(String::as_str) == Some(value))
}

/// Deletes in `output` as `(d, i, p)`.
pub(super) fn deletes(output: &[u8]) -> Vec<(String, u32, Option<u32>)> {
  commands(output)
    .into_iter()
    .filter(|c| c.get("a").is_some_and(|a| a == "d"))
    .map(|c| {
      let number = |key: &str| c.get(key).and_then(|v| v.parse().ok());
      (c["d"].clone(), number("i").unwrap_or(0), number("p"))
    })
    .collect()
}

#[test]
fn centering_moves_resident_tiles() -> Result<()> {
  let mut graphics = Graphics::default();
  let key = tile(&mut graphics, 0)?;
  let mut output = Vec::new();
  graphics.begin(&mut output)?;
  assert!(graphics.place_at((0, key), (48.0, 0.0), SCREEN, 0, &mut output)?);
  graphics.end(&mut output)?;
  let first =
    commands(&output).into_iter().find(|c| has(c, &[("a", "p")])).unwrap();
  output.clear();
  graphics.begin(&mut output)?;
  assert!(graphics.place_at(
    (0, key),
    (52.0, 0.0),
    Screen { columns: SCREEN.columns + 1, ..SCREEN },
    0,
    &mut output
  )?);
  graphics.end(&mut output)?;
  let placed = commands(&output);
  assert_eq!(placed.len(), 1);
  assert!(has(
    &placed[0],
    &[("a", "p"), ("i", &first["i"]), ("p", &first["p"])]
  ));
  assert_ne!(placed[0]["X"], first["X"]);
  assert_eq!(graphics.ids.len(), 1);
  assert_eq!(deletes(&output), []);
  Ok(())
}

#[test]
fn frames_delete_only_what_they_leave() -> Result<()> {
  let mut graphics = Graphics::default();
  let (a, b) = (tile(&mut graphics, 0)?, tile(&mut graphics, 1)?);
  let (id_a, id_b) = (graphics.ids[&(0, a)], graphics.ids[&(0, b)]);
  let rect = Bounds { x0: 0.0, y0: 0.0, x1: 4.0, y1: 4.0 };
  let mut output = Vec::new();
  graphics.begin(&mut output)?;
  graphics.place_at((0, a), (0.0, 0.0), SCREEN, 0, &mut output)?;
  graphics.place_at((0, b), (0.0, 64.0), SCREEN, 0, &mut output)?;
  graphics.highlight(rect, [1; 4], SCREEN, 0, &mut output)?;
  graphics.end(&mut output)?;
  assert_eq!(deletes(&output), []);
  let swatch = graphics.solids[&([1; 4], SWATCH.0, SWATCH.1)];
  // The next frame moves `a` and drops `b` and the highlight.
  output.clear();
  graphics.begin(&mut output)?;
  graphics.place_at((0, a), (0.0, 3.0), SCREEN, 0, &mut output)?;
  graphics.end(&mut output)?;
  let mut deleted = deletes(&output);
  deleted.sort();
  let want = [("i".into(), id_b, Some(1)), ("i".into(), swatch, Some(1))];
  let mut want = want.to_vec();
  want.sort();
  assert_eq!(deleted, want);
  let placed: Vec<_> =
    commands(&output).into_iter().filter(|c| has(c, &[("a", "p")])).collect();
  assert_eq!(placed.len(), 1);
  assert!(has(&placed[0], &[("i", &id_a.to_string()), ("p", "1"), ("Y", "3")]));
  // An unchanged frame deletes nothing.
  output.clear();
  graphics.begin(&mut output)?;
  graphics.place_at((0, a), (0.0, 3.0), SCREEN, 0, &mut output)?;
  graphics.end(&mut output)?;
  assert_eq!(deletes(&output), []);
  Ok(())
}

#[test]
fn preview_keeps_pixels_while_shown() -> Result<()> {
  let mut graphics = Graphics::default();
  let id = graphics.base + PREVIEW;
  let rect = Bounds { x0: 0.0, y0: 0.0, x1: 2.0, y1: 2.0 };
  let mut output = Vec::new();
  for _ in 0..2 {
    output.clear();
    graphics.begin(&mut output)?;
    graphics.picture(7, (2, 2, &[0; 12]), rect, SCREEN, 0, &mut output)?;
    graphics.end(&mut output)?;
    assert_eq!(deletes(&output), []);
  }
  assert!(commands(&output).iter().any(|c| has(c, &[("a", "p"), ("p", "1")])));
  output.clear();
  graphics.begin(&mut output)?;
  graphics.end(&mut output)?;
  assert_eq!(deletes(&output), [("I".into(), id, None)]);
  assert_eq!(graphics.preview, 0);
  // A late `ENOENT` while a newer upload shows: the next picture
  // uploads again, and a frame without one deletes the id.
  for pictured in [true, false] {
    graphics.picture(7, (2, 2, &[0; 12]), rect, SCREEN, 0, &mut output)?;
    graphics.missing(id);
    output.clear();
    graphics.begin(&mut output)?;
    if pictured {
      graphics.picture(7, (2, 2, &[0; 12]), rect, SCREEN, 0, &mut output)?;
      assert!(commands(&output).iter().any(|c| has(c, &[("a", "t")])));
    }
    graphics.end(&mut output)?;
    let deleted = deletes(&output).contains(&("I".into(), id, None));
    assert_eq!(deleted, !pictured);
  }
  Ok(())
}

#[test]
fn missing_images_upload_again() -> Result<()> {
  let mut graphics = Graphics::default();
  let a = tile(&mut graphics, 0)?;
  let id = graphics.ids[&(0, a)];
  let mut output = Vec::new();
  graphics.highlight(
    Bounds { x0: 0.0, y0: 0.0, x1: 4.0, y1: 4.0 },
    [1; 4],
    SCREEN,
    0,
    &mut output,
  )?;
  let swatch = graphics.solids[&([1; 4], SWATCH.0, SWATCH.1)];
  graphics.preview = 3;
  assert!(graphics.missing(id));
  assert!(!graphics.contains(&(0, a)));
  assert!(graphics.missing(swatch));
  assert!(graphics.solids.is_empty());
  assert!(graphics.missing(graphics.base + PREVIEW));
  assert_eq!(graphics.preview, LOST);
  assert!(!graphics.missing(1));
  Ok(())
}

#[test]
fn loss_keeps_encoded_tiles() -> Result<()> {
  let mut graphics = Graphics::default();
  let key = tile(&mut graphics, 0)?;
  let payload = Payload::Encoded(encode(&[255; 64 * 64 * 3])?.into());
  graphics.keep(0, Tile { key, payload, render_ms: 0.0 });
  let rect = Bounds { x0: 0.0, y0: 0.0, x1: 4.0, y1: 4.0 };
  let frame = |graphics: &mut Graphics| -> Result<Vec<u8>> {
    let mut output = Vec::new();
    graphics.begin(&mut output)?;
    graphics.place_at((0, key), (0.0, 0.0), SCREEN, 0, &mut output)?;
    graphics.highlight(rect, [3; 4], SCREEN, 0, &mut output)?;
    graphics.picture(7, (2, 2, &[0; 12]), rect, SCREEN, 0, &mut output)?;
    graphics.end(&mut output)?;
    Ok(output)
  };
  frame(&mut graphics)?;
  let cached = graphics.stored.entries().count();
  graphics.lost();
  assert_eq!(graphics.stored.entries().count(), cached);
  assert!(!graphics.contains(&(0, key)));
  assert_eq!(graphics.shown, []);
  assert!(graphics.solids.is_empty());
  let uploaded = frame(&mut graphics)?;
  let commands = commands(&uploaded);
  assert_eq!(commands.iter().filter(|c| has(c, &[("a", "t")])).count(), 3);
  let reused = frame(&mut graphics)?;
  assert!(!String::from_utf8_lossy(&reused).contains("a=t,"));
  assert!(graphics.contains(&(0, key)));
  Ok(())
}

#[test]
fn placements_report_errors_only_when_asked() -> Result<()> {
  for quiet in [1, 2] {
    let mut graphics = Graphics { quiet, ..Graphics::default() };
    let a = tile(&mut graphics, 0)?;
    let mut output = Vec::new();
    graphics.place_at((0, a), (0.0, 0.0), SCREEN, 0, &mut output)?;
    let q = quiet.to_string();
    assert!(has(&commands(&output)[0], &[("a", "p"), ("q", &q)]));
  }
  Ok(())
}

#[test]
fn tiny_images_skip_compression() -> Result<()> {
  // Fixed streams keep backend sizes irrelevant.
  for (count, encoded, compressed) in [
    (1, "eAH7z8DQAAAEgAGA", false),
    (14, "eAH7z8DQ8J8MDABfshTz", false),
    (
      1000,
      "eAHt0jENAAAMw7BCH/MWxh4fIRC5yVUeMMAAAwwwwAADDDDAAAMMMPBqYMyg2GQ=",
      true,
    ),
  ] {
    let pixels = [255, 0, 0, 128].repeat(count);
    let mut output = Vec::new();
    transmit(
      Transport::default(),
      &mut output,
      99,
      count as i32,
      1,
      32,
      encoded,
    )?;
    let controls = commands(&output);
    assert_eq!(controls[0].contains_key("o"), compressed);
    let text = String::from_utf8(output)?;
    let payload = text.split_once(';').unwrap().1.trim_end_matches("\x1b\\");
    let bytes = STANDARD.decode(payload)?;
    let actual = if compressed {
      let mut decoded = Vec::new();
      ZlibDecoder::new(bytes.as_slice()).read_to_end(&mut decoded)?;
      decoded
    } else {
      bytes
    };
    assert_eq!(actual, pixels);
  }
  Ok(())
}

#[test]
fn negative_overlays_stay_inside_cells() -> Result<()> {
  let mut graphics = Graphics { cell_offsets: true, ..Graphics::default() };
  let rect = Bounds { x0: 3.0, y0: 7.0, x1: 93.0, y1: 133.0 };
  let mut output = Vec::new();
  graphics.backdrop(rect, [1; 4], SCREEN, 0, &mut output)?;
  let mut area = 0;
  for command in commands(&output) {
    if command.get("a").map(String::as_str) != Some("p") {
      continue;
    }
    let number = |key: &str| command[key].parse::<u32>().unwrap();
    let (x, y, w, h) = (number("X"), number("Y"), number("w"), number("h"));
    assert!(x == 0 || x + w <= 8);
    assert!(y == 0 || y + h <= 16);
    area += w * h;
  }
  assert_eq!(area, 90 * 126);
  for (length, origin, cell, limit) in [(126, 7, 16, 64), (199, 3, 80, 64)] {
    let parts = segments(length, origin, cell, limit);
    let mut end = 0;
    for (start, size) in parts {
      assert_eq!(start, end);
      assert!(size > 0 && size <= limit);
      let offset = (origin + start) % cell;
      assert!(offset == 0 || offset + size <= cell);
      end += size;
    }
    assert_eq!(end, length);
  }
  Ok(())
}

#[test]
fn crop_quirk_sizes_overlays() -> Result<()> {
  let mut graphics = Graphics { quirk: true, ..Graphics::default() };
  let mut output = Vec::new();
  let rect = Bounds { x0: 10.0, y0: 20.0, x1: 50.0, y1: 27.0 };
  graphics.highlight(rect, [1; 4], SCREEN, 0, &mut output)?;
  let sent = commands(&output);
  assert!(has(&sent[0], &[("a", "t"), ("s", "40"), ("v", "7")]));
  let placed = &sent[1];
  assert!(has(placed, &[("a", "p"), ("X", "2"), ("Y", "4")]));
  let crop = ["x", "y", "w", "h", "c", "r"];
  assert!(crop.iter().all(|key| !placed.contains_key(*key)));
  Ok(())
}

#[test]
fn pieces_cover_with_whole_cells() {
  // (length, cell) -> pieces
  let cases = [
    (64, 8, vec![(0, 64)]),
    (59, 8, vec![(0, 56), (51, 8)]),
    (512, 14, vec![(0, 504), (498, 14)]),
    (7, 8, vec![]),
  ];
  for (length, cell, want) in cases {
    assert_eq!(pieces(length, cell), want, "{length} {cell}");
  }
}

/// Edge tiles on iTerm2 cover exactly what shows, in whole cells.
#[test]
fn crop_quirk_edge_tiles_leave_no_gap() -> Result<()> {
  let mut graphics = Graphics { quirk: true, ..Graphics::default() };
  let screen = crate::screen::pixels(80, 20, 14, 30);
  let full = tile_sized(&mut graphics, 0, (512, 256))?;
  // Page-end tiles shorter than a cell.
  let low = tile_sized(&mut graphics, 1, (512, 20))?;
  let thin = tile_sized(&mut graphics, 2, (10, 256))?;
  // (tile, origin) -> shown x and y ranges in tile pixels; None is uncut.
  let cases = [
    (full, (0.0, 0.0), None),
    (full, (-5.0, -21.0), Some(((5, 512), (0, 235)))),
    (full, (900.0, 400.0), Some(((0, 220), (86, 256)))),
    (low, (-5.0, 100.0), Some(((5, 512), (0, 20)))),
    (thin, (300.0, -21.0), Some(((0, 10), (0, 235)))),
  ];
  let mut output = Vec::new();
  for (key, origin, want) in cases {
    output.clear();
    graphics.place_at((0, key), origin, screen, 0, &mut output)?;
    let placed = commands(&output);
    let Some((xs, ys)) = want else {
      assert_eq!(placed.len(), 1);
      assert!(!placed[0].contains_key("c"));
      continue;
    };
    let mut spans = (Vec::new(), Vec::new());
    for command in &placed {
      let n = |key: &str| command[key].parse::<u32>().unwrap_or(0);
      // Whole cells, or one stretched cell for a side under a cell.
      let fits = |pixels, cells, size| {
        pixels == cells * size || (cells == 1 && pixels < size)
      };
      assert!(fits(n("w"), n("c"), 14), "{command:?}");
      assert!(fits(n("h"), n("r"), 30), "{command:?}");
      spans.0.push((n("x"), n("x") + n("w")));
      spans.1.push((n("y"), n("y") + n("h")));
    }
    assert_eq!(covered(spans.0), xs, "{origin:?}");
    assert_eq!(covered(spans.1), ys, "{origin:?}");
  }
  // A cut side under a cell draws nothing.
  output.clear();
  graphics.place_at((0, full), (0.0, -250.0), screen, 0, &mut output)?;
  assert_eq!(commands(&output).len(), 0);
  Ok(())
}

#[test]
fn preview_crop_matches_the_image() -> Result<()> {
  let pixels = [0; 2 * 40 * 3];
  // (quirk, origin) -> (w, h, y, cells)
  let cases = [
    (false, (0.5, 0.5), vec![("2", "40", None, None)]),
    // The screen's 144 px cut 24 rows: a cell, then one ending there.
    (
      true,
      (0.0, 120.0),
      vec![
        ("2", "16", Some("24"), Some(("1", "1"))),
        ("2", "16", Some("16"), Some(("1", "1"))),
      ],
    ),
  ];
  for (quirk, (x, y), want) in cases {
    let mut graphics = Graphics { quirk, ..Graphics::default() };
    let rect = Bounds { x0: x, y0: y, x1: x + 2.0, y1: y + 40.0 };
    let mut output = Vec::new();
    graphics.picture(1, (2, 40, &pixels), rect, SCREEN, 0, &mut output)?;
    let placed: Vec<_> =
      commands(&output).into_iter().filter(|c| has(c, &[("a", "p")])).collect();
    assert_eq!(placed.len(), want.len(), "{placed:?}");
    for (command, (w, h, y, cells)) in placed.iter().zip(want) {
      assert!(has(command, &[("w", w), ("h", h)]), "{command:?}");
      assert_eq!(command.get("y").map(String::as_str), y);
      let keys = cells.map(|(c, r)| [("c", c), ("r", r)]);
      match keys {
        Some(keys) => assert!(has(command, &keys), "{command:?}"),
        None => assert!(!command.contains_key("c"), "{command:?}"),
      }
    }
  }
  Ok(())
}

/// The range `spans` cover, which must leave no gap.
fn covered(mut spans: Vec<(u32, u32)>) -> (u32, u32) {
  spans.sort_unstable();
  let mut range = spans[0];
  for (start, end) in spans {
    assert!(start <= range.1, "gap before {start}");
    range.1 = range.1.max(end);
  }
  range
}

/// Every remembered placement names an image vpdf holds.
fn consistent(graphics: &Graphics) {
  let held = |id: u32| {
    graphics.ids.values().any(|&held| held == id)
      || graphics.solids.values().any(|&held| held == id)
      || id == graphics.base + PREVIEW
  };
  for &(id, p) in &graphics.shown {
    assert!(held(id), "{id},{p}");
  }
}

#[test]
fn late_missing_replies_keep_placements_known() -> Result<()> {
  let mut graphics = Graphics::default();
  let key = tile(&mut graphics, 0)?;
  let payload = Payload::Encoded(encode(&[255; 64 * 64 * 3])?.into());
  graphics.keep(0, Tile { key, payload, render_ms: 0.0 });
  let id = graphics.ids[&(0, key)];
  let rect = Bounds { x0: 0.0, y0: 0.0, x1: 4.0, y1: 4.0 };
  let mut output = Vec::new();
  let frame = |graphics: &mut Graphics| -> Result<Vec<u8>> {
    let mut output = Vec::new();
    graphics.begin(&mut output)?;
    graphics.place_at((0, key), (0.0, 0.0), SCREEN, 0, &mut output)?;
    graphics.highlight(rect, [3; 4], SCREEN, 0, &mut output)?;
    graphics.end(&mut output)?;
    Ok(output)
  };
  frame(&mut graphics)?;
  let swatch = graphics.solids[&([3; 4], SWATCH.0, SWATCH.1)];
  // One eviction, two replies each, with a frame in between.
  for _ in 0..2 {
    assert!(graphics.missing(id));
    assert!(graphics.missing(swatch));
    output = frame(&mut graphics)?;
    // Deleted before the ids upload again.
    let deleted = deletes(&output);
    for id in [id, swatch] {
      assert!(deleted.contains(&("I".into(), id, None)), "{id}");
    }
    consistent(&graphics);
  }
  assert!(commands(&output).iter().any(|c| has(c, &[("a", "t")])));
  Ok(())
}

#[test]
fn overlay_ids_recycle_and_run_out_quietly() -> Result<()> {
  let mut graphics = Graphics { quirk: true, ..Graphics::default() };
  let mut output = Vec::new();
  let mut sizes = 0;
  graphics.begin(&mut output)?;
  // One frame wants more sizes than there are overlay ids.
  for size in 0..PREVIEW + 10 {
    let (x1, y1) = ((size % 32 + 1) as f32, (size / 32 + 1) as f32);
    let rect = Bounds { x0: 0.0, y0: 0.0, x1, y1 };
    graphics.highlight(rect, [4; 4], SCREEN, 0, &mut output)?;
    sizes = graphics.solids.len();
  }
  graphics.end(&mut output)?;
  assert!(sizes <= PREVIEW as usize);
  // The next frame frees what it does not place, and ids stay dense.
  graphics.begin(&mut output)?;
  let rect = Bounds { x0: 0.0, y0: 0.0, x1: 3.0, y1: 1.0 };
  graphics.highlight(rect, [5; 4], SCREEN, 0, &mut output)?;
  graphics.end(&mut output)?;
  assert_eq!(graphics.solids.len(), 1);
  let id = graphics.solids[&([5; 4], 3, 1)];
  assert!((graphics.base..graphics.base + PREVIEW).contains(&id));
  assert_eq!(graphics.next_overlay(), graphics.base + PREVIEW);
  consistent(&graphics);
  Ok(())
}

#[test]
fn release_deletes_each_owned_id() -> Result<()> {
  let mut graphics = Graphics::default();
  let a = tile(&mut graphics, 0)?;
  let id = graphics.ids[&(0, a)];
  let mut output = Vec::new();
  graphics.highlight(
    Bounds { x0: 0.0, y0: 0.0, x1: 4.0, y1: 4.0 },
    [2; 4],
    SCREEN,
    0,
    &mut output,
  )?;
  let swatch = graphics.solids[&([2; 4], SWATCH.0, SWATCH.1)];
  output.clear();
  graphics.release(&mut output)?;
  let deleted = deletes(&output);
  assert!(deleted.iter().all(|(d, _, p)| d == "I" && p.is_none()));
  for id in [id, swatch, graphics.base + PREVIEW] {
    assert!(deleted.iter().any(|(_, i, _)| *i == id), "{id}");
  }
  Ok(())
}

#[test]
fn forgetting_a_document_frees_only_its_tiles() -> Result<()> {
  let mut graphics = Graphics::default();
  let mut output = Vec::new();
  for pane in [1, 2] {
    upload(&mut graphics, pane, tile_key(0, (1, 1)), &mut output)?;
  }
  graphics.forget(1);
  output.clear();
  graphics.begin(&mut output)?;
  let deleted = commands(&output)
    .into_iter()
    .filter(|c| c.get("d").is_some_and(|d| d == "I"))
    .count();
  assert_eq!(deleted, 1);
  assert_eq!(graphics.free.len(), 1);
  assert!(graphics.ids.keys().all(|key| key.0 == 2));
  Ok(())
}

#[test]
fn profiles_survive_image_loss() -> Result<()> {
  for tmux in [false, true] {
    let profile = Presentation {
      transport: Transport { tmux },
      crop_quirk: true,
      cell_offsets: true,
      report_missing: true,
    };
    let mut graphics = Graphics::new(profile, &Encoder::default());
    let key = tile(&mut graphics, 0)?;
    graphics.lost();
    assert!(graphics.quirk && graphics.cell_offsets);
    assert_eq!(graphics.quiet, 1);
    tile(&mut graphics, 0)?;
    let mut out = Vec::new();
    graphics.place_at((0, key), (0.0, 0.0), SCREEN, 0, &mut out)?;
    assert_eq!(out.starts_with(b"\x1bPtmux;"), tmux);
    assert!(commands(&out).iter().any(|c| has(c, &[("a", "p"), ("q", "1")])));
    graphics.release(&mut out)?;
    assert!(deletes(&out).iter().any(|(kind, _, _)| kind == "I"));
  }
  Ok(())
}

#[test]
fn closed_spool_requests_raster() -> Result<()> {
  use std::sync::Arc;
  let dir = tempfile::tempdir()?;
  let spool = Arc::new(Spool::create(dir.path())?);
  let encoder = Encoder::new(Some(Arc::clone(&spool)));
  let mut graphics = Graphics::new(Presentation::default(), &encoder);
  assert_eq!(graphics.budget, SPOOLED);
  let key = TileKey { scale: 1024, width: 1, height: 1, ..TileKey::default() };
  graphics
    .keep(0, Tile { key, payload: encoder.encode(&[255; 3])?, render_ms: 0.0 });
  spool.close();
  let mut out = Vec::new();
  assert!(!graphics.place_at((0, key), (0.0, 0.0), SCREEN, 0, &mut out)?);
  assert_eq!(out, [] as [u8; 0]);
  assert!(!graphics.has(&(0, key)));
  assert!(encoder.encode(&[255; 3]).is_err());
  Ok(())
}

#[test]
fn resizing_keeps_tiles_until_replacements() -> Result<()> {
  let mut graphics = Graphics::default();
  let old = tile_sized(&mut graphics, 0, (64, 64))?;
  let screen =
    Screen { origin: (12, 3), ..crate::screen::pixels(8, 9, 10, 10) };
  let mut out = Vec::new();
  graphics.begin(&mut out)?;
  assert!(graphics.place_at((0, old), (0.0, 0.0), screen, 2, &mut out)?);
  graphics.end(&mut out)?;
  let wanted = TileKey { scale: 2, width: 128, height: 128, ..old };
  let old_id = graphics.ids[&(0, old)];
  for origin in [(0.0, 0.0), (-32.0, -32.0)] {
    out.clear();
    graphics.begin(&mut out)?;
    assert!(!graphics.place_at((0, wanted), origin, screen, 2, &mut out)?);
    graphics.end(&mut out)?;
    assert_eq!((graphics.missing_tiles, graphics.stand_ins), (0, 1));
    assert!(!graphics.has(&(0, wanted)));
    let placements: Vec<_> = commands(&out)
      .into_iter()
      .filter(|command| command.get("a").is_some_and(|a| a == "p"))
      .collect();
    assert_eq!(placements.len(), 1);
    assert_eq!(placements[0]["i"], old_id.to_string());
    assert!(placements[0]["z"].parse::<i32>()? < -2);
    assert_eq!(placements[0]["c"], "8");
    assert_eq!(placements[0]["r"], "8");
  }
  graphics.upload(
    0,
    &Tile {
      key: wanted,
      payload: Payload::Encoded(encode(&vec![255; 128 * 128 * 3])?.into()),
      render_ms: 0.0,
    },
    &mut out,
  )?;
  out.clear();
  graphics.begin(&mut out)?;
  assert!(graphics.place_at((0, wanted), (0.0, 0.0), screen, 2, &mut out)?);
  graphics.end(&mut out)?;
  assert_eq!((graphics.missing_tiles, graphics.stand_ins), (0, 0));
  assert!(deletes(&out).iter().any(|(_, id, _)| *id == old_id));
  Ok(())
}

#[test]
fn fallback_requires_matching_document_geometry() -> Result<()> {
  let mut graphics = Graphics::default();
  let old = tile(&mut graphics, 0)?;
  let wanted = TileKey { scale: 2, width: 128, height: 128, ..old };
  let crop = vpdf_model::geometry::Crop([10, 0, 0, 0]);
  for key in [
    (1, wanted),
    (0, TileKey { page: PageId(1), ..wanted }),
    (0, TileKey { rotation: 90, ..wanted }),
    (0, TileKey { crop, ..wanted }),
  ] {
    let mut out = Vec::new();
    graphics.begin(&mut out)?;
    assert!(!graphics.place_at(key, (0.0, 0.0), SCREEN, 0, &mut out)?);
    graphics.end(&mut out)?;
    assert_eq!((graphics.missing_tiles, graphics.stand_ins), (1, 0));
    let count = commands(&out).len();
    assert_eq!(count, 0);
  }
  Ok(())
}

#[test]
fn prefetched_tiles_cover_pending_scales() -> Result<()> {
  let mut graphics = Graphics::default();
  let old =
    TileKey { scale: 1024, width: 64, height: 64, ..TileKey::default() };
  graphics.keep(
    0,
    Tile {
      key: old,
      payload: Payload::Encoded(encode(&vec![255; 64 * 64 * 3])?.into()),
      render_ms: 0.0,
    },
  );
  assert!(!graphics.contains(&(0, old)));
  let pending = TileKey { scale: 1040, width: 65, height: 65, ..old };
  let screen = Screen { origin: (2, 3), ..crate::screen::pixels(7, 8, 10, 10) };
  let mut out = Vec::new();
  graphics.begin(&mut out)?;
  for y in [0, 32] {
    let key = TileKey { y, height: 32, ..pending };
    assert!(!graphics.place_at(
      (0, key),
      (0.0, y as f32),
      screen,
      4,
      &mut out,
    )?);
  }
  graphics.end(&mut out)?;
  assert_eq!((graphics.missing_tiles, graphics.stand_ins), (0, 2));
  assert!(graphics.contains(&(0, old)));
  assert_eq!(graphics.shown.len(), 1);
  let uploads = commands(&out)
    .iter()
    .filter(|c| c.get("a").is_some_and(|a| a == "t"))
    .count();
  assert_eq!(uploads, 1);
  Ok(())
}

#[test]
fn transparent_pictures_keep_alpha() -> Result<()> {
  let mut graphics = Graphics::default();
  let rect = Bounds { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 };
  let mut output = Vec::new();
  graphics.picture(
    900,
    (1, 1, &[20, 40, 60, 80]),
    rect,
    SCREEN,
    0,
    &mut output,
  )?;
  assert!(commands(&output).iter().any(|command| has(command, &[("f", "32")])));
  output.clear();
  graphics.picture(
    900,
    (1, 1, &[20, 40, 60, 80]),
    rect,
    SCREEN,
    0,
    &mut output,
  )?;
  assert!(!commands(&output).iter().any(|command| has(command, &[("a", "t")])));
  Ok(())
}
