//! Single hot functions, timed in isolation, so a change is measured
//! without the noise of whole-reader runs. Arguments filter cases by name.
use std::hint::black_box;
use std::io::{self, Write};
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::solo::Solo;
use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};
use vpdf_model::text::PageId;
use vpdf_ui::layout::Screen;

#[path = "support/hash.rs"]
mod hash;
mod support;
use support::{Case, measure};
#[path = "support/corpus.rs"]
mod corpus;
use corpus::corpus;

/// A 120x40 pane.
const PANE: Screen = Screen {
  columns: 120,
  rows: 40,
  cell_width: 8,
  cell_height: 16,
  origin: (0, 0),
};

/// The corpus paper on a 120x40 pane.
fn paper(text_mode: bool) -> Result<Solo> {
  Solo::open(&corpus("paper.pdf")?, "", text_mode, PANE)
}

/// Hovered citation discovery and labels.
fn link_hover() -> Result<()> {
  use vpdf_ui::pointer::{Hit, Hover, Region};
  let on = |pane, page, index| {
    let hit = Hit::Link(page, index);
    let beneath = Some((page, index));
    Some(Hover { pane, region: Region::Page, hit, beneath })
  };
  let mut app = paper(false)?;
  app.command("2")?;
  let page = app.navigation_position().page;
  anyhow::ensure!(app.doc.links(page)?.len() >= 2, "citations on page 2");
  let hover = on(app.id, page, 0);
  app.point(hover);
  measure("App::hover_label", || {
    black_box(app.hover_label());
  })?;
  Ok(())
}

/// Rows each frame clips page graphics to while clues show.
fn menu_rows() -> Result<()> {
  let mut app = paper(false)?;
  let prefix = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
  app.key(prefix, &mut vpdf_ui::prompt::History::default())?;
  app
    .overlays
    .clues_mut()
    .restart(Instant::now().checked_sub(Duration::from_secs(1)));
  let _ = app.clue_due();
  assert!(app.overlays.clues().shown().is_some());
  measure("App::menu_rows clues", || {
    black_box(app.menu_rows());
  })
}

/// Motion along a citation-dense row.
fn hover() -> Result<()> {
  use vpdf_model::input::{MouseEvent, MouseEventKind};
  use vpdf_ui::pointer::Region;
  let motion = |column| MouseEvent {
    kind: MouseEventKind::Moved,
    column,
    row: 20,
    modifiers: KeyModifiers::NONE,
  };
  let mut session = vpdf::session::Session::new(None);
  session.open(&corpus("paper.pdf")?, true, PANE)?;
  session.app().command("2")?;
  let mut column = 0;
  measure("Session::mouse motion", || {
    column = (column + 1) % 120;
    black_box(session.mouse(black_box(motion(column))).expect("hover"));
  })?;
  for text_mode in [false, true] {
    let mut app = paper(text_mode)?;
    app.command("2")?;
    app.draw(&mut Vec::new())?;
    let mut column = 0;
    measure(&format!("App::hovered text={text_mode}"), || {
      column = (column + 1) % 120;
      let hit = app.hovered(Region::Page, black_box(column), 20, None);
      black_box(hit.expect("hit test"));
    })?;
  }
  Ok(())
}

/// Wheel turns over the page: scroll, Ctrl zoom, and Shift pan.
fn wheel() -> Result<()> {
  use vpdf_model::input::{MouseEvent, MouseEventKind};
  let turn =
    |kind, modifiers| MouseEvent { kind, column: 60, row: 20, modifiers };
  let mut session = vpdf::session::Session::new(None);
  session.open(&corpus("paper.pdf")?, true, PANE)?;
  session.app().command("2")?;
  let cases = [
    ("scroll", KeyModifiers::NONE),
    ("ctrl zoom", KeyModifiers::CONTROL),
    ("shift pan", KeyModifiers::SHIFT),
  ];
  for (name, modifiers) in cases {
    if modifiers == KeyModifiers::SHIFT {
      session.app().layout.request_zoom(vpdf_ui::layout::Zoom::To(2.0));
      session.app().relayout()?;
    }
    measure(&format!("Session::mouse wheel {name}"), || {
      for kind in [MouseEventKind::ScrollUp, MouseEventKind::ScrollDown] {
        black_box(
          session.mouse(black_box(turn(kind, modifiers))).expect("wheel"),
        );
      }
    })?;
  }
  Ok(())
}

/// Row drags: plain, by word, edge.
fn drag() -> Result<()> {
  use vpdf_model::input::{MouseButton, MouseEvent, MouseEventKind};
  let event = |kind, column| MouseEvent {
    kind,
    column,
    row: 20,
    modifiers: KeyModifiers::NONE,
  };
  for clicks in [1, 2] {
    let mut session = vpdf::session::Session::new(None);
    session.open(&corpus("paper.pdf")?, true, PANE)?;
    session.app().command("2")?;
    let press = MouseEventKind::Down(MouseButton::Left);
    for _ in 1..clicks {
      session.mouse(event(press, 40))?;
      session.mouse(event(MouseEventKind::Up(MouseButton::Left), 40))?;
    }
    session.mouse(event(press, 40))?;
    let mut column = 40;
    measure(&format!("Session::mouse drag clicks={clicks}"), || {
      column = if column == 80 { 50 } else { 80 };
      let drag = event(MouseEventKind::Drag(MouseButton::Left), column);
      black_box(session.mouse(black_box(drag)).expect("drag"));
    })?;
  }
  // Middle drags pan a zoomed page.
  let mut session = vpdf::session::Session::new(None);
  session.open(&corpus("paper.pdf")?, true, PANE)?;
  session.app().command("2")?;
  session.app().layout.request_zoom(vpdf_ui::layout::Zoom::To(3.0));
  session.app().relayout()?;
  session.mouse(event(MouseEventKind::Down(MouseButton::Middle), 60))?;
  let mut column = 60;
  measure("Session::mouse drag middle", || {
    column = if column == 70 { 50 } else { 70 };
    let drag = event(MouseEventKind::Drag(MouseButton::Middle), column);
    black_box(session.mouse(black_box(drag)).expect("pan"));
  })?;
  // Held at the status row: no scroll.
  let mut session = vpdf::session::Session::new(None);
  session.open(&corpus("paper.pdf")?, true, PANE)?;
  session.app().command("2")?;
  let base = session.app().reader.scroll;
  session.mouse(event(MouseEventKind::Down(MouseButton::Left), 40))?;
  let mut column = 40;
  measure("Session::mouse drag held edge", || {
    column = if column == 80 { 50 } else { 80 };
    let drag = MouseEvent {
      row: 39,
      ..event(MouseEventKind::Drag(MouseButton::Left), column)
    };
    black_box(session.mouse(black_box(drag)).expect("drag"));
  })?;
  // A step scrolls a row and extends.
  let mut now = session.autoscroll_due().expect("a held edge drag");
  measure("Session::autoscroll step", || {
    now += Duration::from_millis(50);
    session.app().reading_view().scrub_scroll(base);
    black_box(session.autoscroll(black_box(now)));
  })
}

/// A warm frame of the corpus paper, scrolled 4 px from the last one,
/// with every visible tile resident; prints the frame's bytes too.
fn frame() -> Result<()> {
  let mut app = paper(false)?;
  let base = app.reader.scroll;
  for scroll in [base, base + 4.0] {
    for key in vpdf_ui::layout::visible(&app.layout, scroll) {
      if !app.graphics.contains(&(app.id, key)) {
        let tile = vpdf::raster::render(&mut app.doc, key)?;
        app.graphics.upload(app.id, &tile, &mut io::sink())?;
      }
    }
  }
  let mut out = Vec::with_capacity(64 * 1024);
  let mut step = 0_u32;
  let mut draw = |out: &mut Vec<u8>| {
    app
      .reading_view()
      .scrub_scroll(base + f32::from(u8::from(step % 2 == 1)) * 4.0);
    step += 1;
    out.clear();
    app.draw(out).expect("resident tiles");
  };
  draw(&mut out);
  draw(&mut out);
  writeln!(io::stdout(), "App::draw bytes {}", out.len())?;
  println!("App::draw output {}", hash::digest(&out));
  measure("App::draw scroll", || {
    draw(&mut out);
    black_box(&out);
  })
}

/// Two panes, the pointer resting on one.
fn split_frame() -> Result<()> {
  use vpdf::trace::Drawable;
  use vpdf_model::input::{MouseEvent, MouseEventKind};
  let mut session = vpdf::session::Session::new(None);
  session.open(&corpus("paper.pdf")?, true, PANE)?;
  session.app().command("vsplit")?;
  session.settle()?;
  let rest = |column| MouseEvent {
    kind: MouseEventKind::Moved,
    column,
    row: 20,
    modifiers: KeyModifiers::NONE,
  };
  session.mouse(rest(30))?;
  let mut out = Vec::with_capacity(64 * 1024);
  session.draw_frame(&mut out)?;
  println!("Session::draw_frame output {}", hash::digest(&out));
  measure("Session::draw_frame split", || {
    out.clear();
    session.draw_frame(&mut out).expect("warm frame");
    black_box(&out);
  })
}

fn viewport() -> Result<()> {
  for text_mode in [false, true] {
    let mut app = paper(text_mode)?;
    app.command("4")?;
    app.ensure_cursor()?;
    let cursor = app.reader.cursor;
    let scroll = app.reader.scroll;
    measure(&format!("App::scroll text={text_mode}"), || {
      app.reading_view().scrub_scroll(scroll);
      app.scroll(16.0).expect("scroll");
      app.scroll(-16.0).expect("scroll");
    })?;
    let mut narrow = false;
    measure(&format!("App::relayout text={text_mode}"), || {
      app.reading_view().scrub_scroll(scroll);
      narrow = !narrow;
      app.screen.columns = if narrow { 100 } else { 120 };
      app.relayout().expect("layout");
    })?;
    anyhow::ensure!(app.reader.cursor == cursor, "Viewport cursor drifted");
  }
  Ok(())
}

/// `zc` on, then off, at page 100 of a book.
fn crop_toggle() -> Result<()> {
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = Solo::open(&corpus("book.pdf")?, "", false, screen)?;
  app.command("100")?;
  app.toggle_crop()?;
  let started = Instant::now();
  while app.crop_scan.active() {
    app.crop_updates()?;
    anyhow::ensure!(
      started.elapsed() < Duration::from_secs(30),
      "Crop timeout"
    );
    std::thread::sleep(Duration::from_millis(1));
  }
  app.toggle_crop()?;
  measure("App::toggle_crop on, off", || {
    black_box(app.toggle_crop()).expect("crop toggle");
    black_box(app.toggle_crop()).expect("crop toggle");
  })
}

fn routing_key() -> Result<()> {
  let mut app = paper(false)?;
  let mut history = vpdf_ui::prompt::History::default();
  let key = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
  measure("App::key reading", || {
    app.reading_view().scrub_scroll(0.0);
    app.key(black_box(key), &mut history).expect("reading key");
  })
}

fn cursor_key() -> Result<()> {
  let mut app = paper(false)?;
  let mut history = vpdf_ui::prompt::History::default();
  app
    .key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE), &mut history)?;
  let cursor = app.reader.cursor;
  let key = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE);
  let back = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE);
  measure("App::key Cursor glyph pair", || {
    app.key(black_box(key), &mut history).expect("glyph key");
    app.key(black_box(back), &mut history).expect("return glyph key");
  })?;
  anyhow::ensure!(app.reader.cursor == cursor, "Cursor pair drifted");
  Ok(())
}

fn preview_idle() -> Result<()> {
  let mut app = paper(false)?;
  measure("App::draw_preview absent", || {
    let Solo { app, graphics } = black_box(&mut app);
    app.draw_preview(graphics, &mut io::sink()).expect("no preview");
  })
}

fn preview_scene(text: bool) -> Result<Solo> {
  use vpdf_model::document::Target;
  let mut app = paper(text)?;
  let link = app
    .doc
    .links(PageId(1))?
    .iter()
    .find(|link| !matches!(link.target, Target::Uri(_)))
    .cloned()
    .expect("citation");
  let position = app.doc.text(PageId(1))?.nearest(
    PageId(1),
    f32::midpoint(link.first().x0, link.first().x1),
    f32::midpoint(link.first().y0, link.first().y1),
  );
  app.go_to(position, true)?;
  app.preview_link()?;
  Ok(app)
}

fn preview_live() -> Result<()> {
  for text in [false, true] {
    let mut app = preview_scene(text)?;
    let mut history = vpdf_ui::prompt::History::default();
    let key = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
    measure(&format!("preview prefix text={text}"), || {
      app.keys.clear();
      app.key(black_box(key), &mut history).expect("preview prefix");
    })?;
    app.keys.clear();
    measure(&format!("preview scroll text={text}"), || {
      black_box(&mut app).scroll_preview(black_box(1.0)).expect("preview down");
      black_box(&mut app).scroll_preview(black_box(-1.0)).expect("preview up");
    })?;
    // Warm: pages stay cached between runs.
    measure(&format!("preview reopen cross text={text}"), || {
      app.overlays.close_preview();
      app.preview_link().expect("preview");
      black_box(&mut app).scroll_preview(black_box(40.0)).expect("down");
    })?;
  }
  Ok(())
}

fn routing_pointer() -> Result<()> {
  use vpdf_model::input::{MouseEvent, MouseEventKind};
  let app = paper(false)?;
  let mut session = vpdf::session::Session::new(None);
  session.open(&app.path, true, app.screen)?;
  let mouse = MouseEvent {
    kind: MouseEventKind::ScrollDown,
    column: 80,
    row: 12,
    modifiers: KeyModifiers::NONE,
  };
  measure("Session::mouse wheel", || {
    session.app().reading_view().scrub_scroll(0.0);
    session.mouse(black_box(mouse)).expect("reading wheel");
  })
}

fn routing_outline() -> Result<()> {
  let mut app = paper(false)?;
  app.open_panel(vpdf_ui::keys::PanelKind::Outline)?;
  app.panel.as_mut().expect("outline").view.focus(false);
  measure("App::track_outline", || {
    app.track_outline();
    black_box(app.outline.current());
  })?;
  // Landed views match headings by glyph.
  app.doc.load_outline()?;
  let target = app.doc.outline[5].target.clone();
  app.follow(&target)?;
  measure("App::track_outline landed", || {
    app.track_outline();
    black_box(app.outline.current());
  })
}

fn routing_panel_press() -> Result<()> {
  use vpdf_model::input::{MouseButton, MouseEvent, MouseEventKind};
  let mut app = paper(false)?;
  app.open_panel(vpdf_ui::keys::PanelKind::Help)?;
  let mouse = MouseEvent {
    kind: MouseEventKind::Down(MouseButton::Left),
    column: 2,
    row: 2,
    modifiers: KeyModifiers::NONE,
  };
  measure("App::panel_mouse press", || {
    app.panel_mouse(black_box(mouse), 1).expect("help row press");
  })
}

fn transitions() -> Result<()> {
  use vpdf_model::input::{MouseButton, MouseEvent, MouseEventKind};
  let app = paper(false)?;
  let mut session = vpdf::session::Session::new(None);
  session.open(&app.path, true, app.screen)?;
  session.app().command("vsplit")?;
  session.settle()?;
  measure("Session::mouse pane press", || {
    let column = if session.app().at.column == 0 { 90 } else { 10 };
    let mouse = MouseEvent {
      kind: MouseEventKind::Down(MouseButton::Left),
      column,
      row: 39,
      modifiers: KeyModifiers::NONE,
    };
    session.mouse(black_box(mouse)).expect("pane press");
  })?;
  session.open(&corpus("book.pdf")?, true, app.screen)?;
  measure("Session::activate", || {
    session.activate(black_box(1 - session.active)).expect("tab activation");
  })?;
  let mut app = paper(false)?;
  measure("App::blur", || app.blur().expect("pane blur"))
}

const CASES: &[(&str, Case)] = &[
  ("hover", hover),
  ("wheel", wheel),
  ("crop toggle", crop_toggle),
  ("menu_rows", menu_rows),
  ("link hover", link_hover),
  ("drag", drag),
  ("frame", frame),
  ("viewport", viewport),
  ("transitions", transitions),
  ("routing key", routing_key),
  ("cursor key", cursor_key),
  ("preview absent", preview_idle),
  ("preview live", preview_live),
  ("split frame", split_frame),
  ("routing pointer", routing_pointer),
  ("routing fixes outline", routing_outline),
  ("routing fixes panel press", routing_panel_press),
];

fn main() -> Result<()> {
  support::run(CASES)
}
