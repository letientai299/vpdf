use super::*;
use crate::text::{Bounds, Glyph, Line, PageId, Text};
use std::sync::Arc;
use vpdf_model::geometry::At;
use vpdf_reader::reader::{Location, Operator, SelectionKind};

struct Source {
  text: Arc<Text>,
  reads: usize,
  fail: usize,
  headings: Vec<vpdf_model::document::Heading>,
  links: Vec<vpdf_model::document::Link>,
  notes: Vec<vpdf_model::document::Annotation>,
  lookups: Vec<&'static str>,
  fail_lookup: usize,
}

impl Pages for Source {
  fn count(&self) -> usize {
    1
  }
  fn bounds(&self, _: PageId) -> Bounds {
    Bounds { x0: 0.0, y0: 0.0, x1: 100.0, y1: 1000.0 }
  }
  fn label(&self, _: PageId) -> &'static str {
    "1"
  }
}

impl TextSource for Source {
  fn page_count(&self) -> usize {
    1
  }
  fn page_text(&mut self, _: PageId) -> Result<Arc<Text>> {
    self.reads += 1;
    anyhow::ensure!(self.reads != self.fail, "Text unavailable");
    Ok(Arc::clone(&self.text))
  }
}

struct Fixture {
  reader: Reader,
  target: Option<Position>,
  animation: Scroll,
  landing: crate::page::landing::Landing,
  layout: Layout,
  text: TextView,
  frame: Frame,
  source: Source,
}

impl Fixture {
  fn new() -> Self {
    let lines = (0..30)
      .map(|row| Line {
        paragraph: false,
        glyphs: vec![Glyph {
          ch: 'a',
          bounds: Bounds {
            x0: 10.0,
            x1: 20.0,
            y0: 10.0 + row as f32 * 20.0,
            y1: 20.0 + row as f32 * 20.0,
          },
        }],
      })
      .collect();
    let source = Source {
      text: Arc::new(Text { lines, truncated: false }),
      reads: 0,
      fail: 0,
      headings: Vec::new(),
      links: Vec::new(),
      notes: Vec::new(),
      lookups: Vec::new(),
      fail_lookup: 0,
    };
    let screen = Screen {
      columns: 10,
      rows: 10,
      cell_width: 10,
      cell_height: 10,
      origin: (0, 0),
    };
    let frame = Frame {
      pane: paint::Frame { screen, at: At::default(), left: 0, text: false },
      visible: screen,
    };
    Self {
      reader: Reader::default(),
      target: None,
      animation: Scroll::default(),
      landing: crate::page::landing::Landing::default(),
      layout: Layout::new([source.bounds(PageId(0))], screen),
      text: TextView::default(),
      frame,
      source,
    }
  }

  fn view(&mut self) -> View<'_, &mut Source> {
    View {
      reader: &mut self.reader,
      target: &mut self.target,
      animation: &mut self.animation,
      landing: &mut self.landing,
      layout: &mut self.layout,
      text: &mut self.text,
      frame: self.frame,
      source: &mut self.source,
    }
  }
}

#[test]
fn scroll_failures_preserve_effect_order() {
  for (fail, scroll, target) in [(1, 200.0, None), (2, 240.0, Some(240.0))] {
    let mut fixture = Fixture::new();
    fixture.reader.enter_cursor();
    fixture.reader.scroll = 50.0;
    fixture.reader.desired = Some(9);
    fixture.reader.operator = Some(Operator { count: 1, register: '"' });
    fixture.reader.message = "pending".into();
    fixture.source.fail = fail;
    fixture.animation.start(200.0, Instant::now());
    assert!(fixture.view().scroll(40.0, ScrollInput::Wheel).is_err());
    assert_eq!(fixture.source.reads, fail);
    assert_eq!(fixture.reader.scroll, scroll);
    assert_eq!(fixture.animation.target(), target);
    assert!(fixture.reader.operator.is_none());
    assert_eq!(fixture.reader.message, "");
    assert_eq!(fixture.reader.desired, Some(9));
    assert_eq!(fixture.target, None);
  }
}

#[test]
fn wheel_keeps_visual_selection_stationary() -> Result<()> {
  for keyboard in [false, true] {
    let mut fixture = Fixture::new();
    fixture.reader.visual(SelectionKind::Character);
    let selected = fixture.reader.selection;
    fixture.reader.scroll = 50.0;
    fixture.animation.start(200.0, Instant::now());
    let input = if keyboard {
      ScrollInput::Keyboard { follow: true }
    } else {
      ScrollInput::Wheel
    };
    fixture.view().scroll(40.0, input)?;
    assert_eq!(fixture.reader.scroll, 50.0);
    assert_eq!(fixture.animation.target(), Some(240.0));
    assert_eq!(fixture.reader.selection, selected);
    if keyboard {
      assert!(fixture.source.reads > 0);
      assert_ne!(fixture.reader.cursor, Position::default());
      assert_eq!(fixture.target, Some(fixture.reader.cursor));
    } else {
      assert_eq!(fixture.source.reads, 0);
      assert_eq!(fixture.reader.cursor, Position::default());
      assert_eq!(fixture.target, None);
    }
  }
  Ok(())
}

#[test]
fn text_uses_full_pane_height() -> Result<()> {
  let mut fixture = Fixture::new();
  fixture.frame.visible.rows = 2;
  let position = Position { line: 4, ..Position::default() };
  assert!(!fixture.view().visible(position)?);
  fixture.frame.pane.text = true;
  assert!(fixture.view().visible(position)?);
  fixture.view().scroll(16.0, ScrollInput::Wheel)?;
  assert_eq!(fixture.text.top().line, 2);
  assert!(!fixture.animation.active());
  Ok(())
}

#[test]
fn reveal_cancels_before_source_failure() {
  for text in [false, true] {
    for operation in ["reveal", "land", "align"] {
      let mut fixture = Fixture::new();
      fixture.frame.pane.text = text;
      fixture.source.fail = 1;
      fixture.animation.start(200.0, Instant::now());
      let result = match operation {
        "reveal" => fixture.view().reveal(),
        "land" => fixture.view().land(),
        _ => fixture.view().align(0.5),
      };
      assert!(result.is_err(), "{operation}");
      assert_eq!(fixture.source.reads, 1);
      assert_eq!(fixture.animation.active(), text && operation != "reveal");
    }
  }
}

fn input(action: crate::keys::Action) -> crate::keys::Input {
  crate::keys::Input { action, count: 1, explicit: false, register: '"' }
}

#[test]
fn normal_horizontal_edges() -> Result<()> {
  use crate::find::Search;
  use crate::keys::{Grammar, Keys};

  for text in [false, true] {
    for wide in [false, true] {
      let mut fixture = Fixture::new();
      fixture.frame.pane.text = text;
      if text {
        let source = Arc::make_mut(&mut fixture.source.text);
        let glyph = source.lines[0].glyphs[0].clone();
        source.lines[0].glyphs = vec![glyph; if wide { 25 } else { 5 }];
        fixture.view().scroll(0.0, ScrollInput::Wheel)?;
      } else {
        fixture.source.fail = 1;
        if wide {
          fixture.layout.request_zoom(crate::layout::Zoom::To(2.0));
          fixture.layout.reflow([fixture.source.bounds(PageId(0))]);
        }
      }
      fixture.reader.cursor.line = 3;
      fixture.reader.scroll = 40.0;
      fixture.target = Some(fixture.reader.cursor);
      let origin = fixture.reader.cursor;
      let top = fixture.text.top();
      let mut keys = Keys::default();
      for (key, right) in [("$", true), ("$", true), ("^", false), ("^", false)]
      {
        let event = crate::keys::parse(key).unwrap()[0];
        let input = keys.feed(event, Grammar::Normal).unwrap();
        assert!(fixture.view().navigate(input, &mut Search::default())?);
        let expected = match (right && wide, text) {
          (true, true) => 15.0,
          (true, false) => fixture.layout.pan_limit(PageId(0)),
          _ => 0.0,
        };
        let actual = if text {
          fixture.text.top().glyph as f32
        } else {
          fixture.layout.pan()
        };
        assert_eq!(actual, expected, "{key}, text={text}, wide={wide}");
        assert_eq!(fixture.reader.mode(), Mode::Normal);
        assert_eq!(fixture.reader.cursor, origin);
        assert_eq!(fixture.reader.scroll, 40.0);
        assert_eq!(fixture.target, Some(origin));
        assert_eq!(fixture.text.top().line, top.line);
        if !text {
          assert_eq!(fixture.source.reads, 0);
        }
      }
    }
  }
  Ok(())
}

#[test]
fn boundary_records_before_reveal_failure() {
  use crate::find::Search;
  use crate::keys::Action;
  use vpdf_reader::motion::Motion;

  for fail in [1, 2] {
    let mut fixture = Fixture::new();
    fixture.reader.enter_cursor();
    fixture.reader.desired = Some(9);
    fixture.animation.start(200.0, Instant::now());
    fixture.source.fail = fail;
    let mut search = Search::default();
    let result =
      fixture.view().navigate(input(Action::Motion(Motion::Last)), &mut search);
    assert!(result.is_err());
    assert_eq!(fixture.source.reads, fail);
    assert_eq!(fixture.reader.desired, None);
    assert_eq!(fixture.reader.jumps().count(), 1);
    let saved = fixture.reader.recall(false, Location::default()).unwrap();
    assert_eq!(saved.anchor.is_some(), fail == 2);
    assert_eq!(fixture.animation.active(), fail == 1);
    assert_eq!(fixture.reader.cursor.line, if fail == 1 { 0 } else { 29 });
  }
}

#[test]
fn selection_actions_skip_reveal() -> Result<()> {
  use crate::find::Search;
  use crate::keys::Action;

  for action in [Action::Visual(SelectionKind::Line), Action::Reselect] {
    let mut fixture = Fixture::new();
    fixture.reader.enter_cursor();
    fixture.animation.start(200.0, Instant::now());
    fixture.source.fail = 1;
    assert!(fixture.view().navigate(input(action), &mut Search::default())?);
    assert_eq!(fixture.source.reads, 0);
    assert!(fixture.animation.active());
  }
  Ok(())
}

#[test]
fn normal_scroll_forgets_search_yank_before_failure() {
  use crate::find::Search;
  use crate::keys::Action;
  use vpdf_reader::motion::Motion;
  use vpdf_search::search::Hit;

  for motion in [Motion::Down, Motion::Right] {
    let mut fixture = Fixture::new();
    fixture.frame.pane.text = true;
    fixture.source.fail = 1;
    fixture.reader.message = "pending".into();
    fixture.animation.start(200.0, Instant::now());
    let mut search = Search::default();
    let found = Position { line: 1, ..Position::default() };
    search.show(
      vpdf_search::query::Query::default(),
      vec![Hit { first: found, last: found, excerpt: "a".into() }],
    );
    search
      .yank_after(Location::default(), Operator { count: 1, register: 'a' });
    assert!(
      fixture
        .view()
        .navigate(input(Action::Motion(motion)), &mut search)
        .is_err()
    );
    let scroll = motion == Motion::Down;
    assert_eq!(search.take_yank().is_none(), scroll);
    assert_eq!(fixture.reader.message, "pending");
    assert_eq!(fixture.animation.active(), !scroll);
  }
}

#[test]
fn location_failure_cancels_before_clamping() {
  for text in [false, true] {
    let mut fixture = Fixture::new();
    fixture.frame.pane.text = text;
    fixture.source.fail = 1;
    fixture.reader.desired = Some(9);
    fixture.animation.start(200.0, Instant::now());
    let position = Position { line: 5, ..Position::default() };
    let location = Location {
      cursor: position,
      target: Some(position),
      scroll: 200.0,
      ..Location::default()
    };
    assert!(fixture.view().restore_location(location).is_err());
    assert!(!fixture.animation.active());
    assert_eq!(fixture.reader.cursor, Position::default());
    assert_eq!(fixture.reader.scroll, 0.0);
    assert_eq!(fixture.reader.desired, Some(9));
    assert_eq!(fixture.target, None);
  }
}

fn sections() -> Sections {
  let points = [70.0, 110.0, 150.0].map(|y| {
    vpdf_model::document::Target::Point { page: PageId(0), x: 10.0, y }
  });
  Sections::new(points.iter(), 1)
}

#[test]
fn section_reads_all_candidates_before_selection() -> Result<()> {
  for (forward, count, y) in [
    (true, 0, 70.0),
    (true, 1, 70.0),
    (true, 99, 150.0),
    (false, 1, 150.0),
    (false, 99, 70.0),
  ] {
    let mut fixture = Fixture::new();
    fixture.reader.enter_cursor();
    fixture.reader.cursor.line = if forward { 0 } else { 29 };
    let origin = fixture.reader.cursor;
    let chosen = fixture.view().section(sections(), forward, count)?;
    assert_eq!(
      chosen.unwrap().target(),
      vpdf_model::document::Target::Point { page: PageId(0), x: 10.0, y }
    );
    assert_eq!(fixture.source.reads, 3);
    assert_eq!(fixture.reader.cursor, origin);
    fixture.source.reads = 0;
    fixture.source.fail = 2;
    assert!(fixture.view().section(sections(), forward, count).is_err());
    assert_eq!(fixture.source.reads, 2);
    assert_eq!(fixture.reader.cursor, origin);
    assert_eq!(fixture.reader.jumps().count(), 0);
  }
  Ok(())
}

#[test]
fn section_landing_requires_visibility_and_no_animation() -> Result<()> {
  for animating in [false, true] {
    let mut fixture = Fixture::new();
    fixture.target = Some(Position::default());
    fixture.source.fail = 1;
    if animating {
      fixture.animation.start(200.0, Instant::now());
    }
    let landed = fixture.view().landed_view(Position::default());
    fixture.landing.record(landed);
    let result = fixture.view().section(sections(), true, 1);
    assert_eq!(result.is_ok(), animating);
    assert_eq!(fixture.source.reads, usize::from(!animating));
    if animating {
      assert!(result?.is_some());
    }
  }
  Ok(())
}

#[test]
fn section_finish_keeps_original_mode() -> Result<()> {
  for fail in [false, true] {
    let mut fixture = Fixture::new();
    fixture.frame.pane.text = true;
    let chosen = fixture.view().section(sections(), true, 1)?.unwrap();
    fixture.reader.enter_cursor();
    fixture.reader.cursor.line = 5;
    fixture.animation.start(200.0, Instant::now());
    if fail {
      fixture.source.fail = fixture.source.reads + 1;
      assert!(fixture.view().land_section(&chosen).is_err());
      assert_eq!(fixture.text.top(), Position::default());
      assert!(fixture.animation.active());
      continue;
    }
    fixture.view().land_section(&chosen)?;
    assert_eq!(fixture.text.top().line, 5);
    assert!(fixture.animation.active());
    assert!(fixture.landing.matches(crate::page::landing::Landed::new(
      fixture.reader.cursor,
      fixture.reader.scroll,
      fixture.layout.pan(),
      fixture.text.top(),
    )));
  }
  Ok(())
}

#[test]
fn jump_failures_preserve_completed_transitions() {
  use crate::page::landing::Span;

  for fail in [1, 2, 3] {
    let mut fixture = Fixture::new();
    fixture.reader.enter_cursor();
    fixture.reader.desired = Some(9);
    fixture.source.fail = fail;
    fixture.animation.start(200.0, Instant::now());
    let old = Position { line: 1, ..Position::default() };
    fixture.landing.show(old, Span::Line, Instant::now());
    let position = Position { line: 5, ..Position::default() };
    assert!(fixture.view().land_at(position).is_err());
    let moved = fail == 3;
    assert_eq!(fixture.source.reads, fail);
    assert_eq!(fixture.reader.jumps().count(), usize::from(moved));
    assert_eq!(
      fixture.reader.cursor,
      if moved { position } else { Position::default() }
    );
    assert_eq!(fixture.reader.desired, if moved { None } else { Some(9) });
    assert_eq!(fixture.target, moved.then_some(position));
    assert_eq!(fixture.animation.active(), !moved);
    assert_eq!(fixture.landing.position(), Some(old));
    assert!(!fixture.view().matches_landing(position));
  }
}

#[test]
fn page_jump_reads_text_before_cancelling_animation() {
  for text in [false, true] {
    let mut fixture = Fixture::new();
    fixture.frame.pane.text = text;
    fixture.reader.enter_cursor();
    fixture.source.fail = 2;
    fixture.animation.start(200.0, Instant::now());
    assert!(fixture.view().go_to_page(PageId(0)).is_err());
    assert_eq!(fixture.reader.jumps().count(), 1);
    assert_eq!(fixture.target, Some(Position::default()));
    assert_eq!(fixture.animation.active(), text);
    assert_eq!(fixture.landing.position(), None);
  }
}

#[test]
fn repeated_landing_only_clamps_and_flashes() -> Result<()> {
  let mut fixture = Fixture::new();
  fixture.reader.enter_cursor();
  let position = Position { line: 5, ..Position::default() };
  fixture.view().land_at(position)?;
  let jumps = fixture.reader.jumps().count();
  fixture.source.reads = 0;
  fixture.source.fail = 2;
  fixture.animation.start(200.0, Instant::now());
  fixture.landing.clear_flash();
  fixture.view().land_at(position)?;
  assert_eq!(fixture.source.reads, 1);
  assert_eq!(fixture.reader.jumps().count(), jumps);
  assert!(fixture.animation.active());
  assert_eq!(fixture.landing.position(), Some(position));
  Ok(())
}

#[test]
fn page_destination_avoids_text_loading() -> Result<()> {
  let mut fixture = Fixture::new();
  fixture.source.fail = 1;
  assert_eq!(
    fixture.view().destination(PageId(999), None)?,
    Position::default()
  );
  assert_eq!(fixture.source.reads, 0);
  assert!(
    fixture.view().destination(PageId(999), Some((10.0, 110.0))).is_err()
  );
  assert_eq!(fixture.source.reads, 1);
  assert_eq!(fixture.reader.cursor, Position::default());
  Ok(())
}

#[test]
fn failed_search_yank_is_consumed_without_restoring() -> Result<()> {
  use crate::find::Search;
  use vpdf_search::{query::Query, search::Hit};

  let mut fixture = Fixture::new();
  fixture.source.fail = 1;
  fixture.animation.start(200.0, Instant::now());
  let target = Position { line: 8, ..Position::default() };
  let mut search = Search::default();
  search.show(
    Query::default(),
    vec![Hit { first: target, last: target, excerpt: "a".into() }],
  );
  let origin = Location {
    cursor: Position { line: 3, ..Position::default() },
    scroll: 50.0,
    ..Location::default()
  };
  search.yank_after(origin, Operator { count: 1, register: 'a' });
  assert!(fixture.view().finish_search_yank(&mut search).is_err());
  assert_eq!(fixture.reader.cursor, target);
  assert_eq!(fixture.reader.scroll, 0.0);
  assert!(fixture.animation.active());
  fixture.view().finish_search_yank(&mut search)?;
  assert_eq!(fixture.source.reads, 1);
  assert!(fixture.reader.pending_yank.is_none());
  Ok(())
}

#[test]
fn owned_reader_binds_navigation() -> Result<()> {
  use crate::find::Search;
  use crate::keys::Action;
  use vpdf_reader::motion::Motion;

  let mut fixture = Fixture::new();
  fixture.reader.enter_cursor();
  let mut pane =
    Pane::new(std::mem::take(&mut fixture.reader), Tracking::default());
  let mut view = pane.bind(
    &mut fixture.layout,
    &mut fixture.text,
    fixture.frame,
    &mut fixture.source,
  );
  assert!(
    view
      .navigate(input(Action::Motion(Motion::Down)), &mut Search::default())?
  );
  assert_eq!(pane.cursor.line, 1);
  assert_eq!(pane.mode(), Mode::Cursor);
  Ok(())
}

#[test]
fn content_cancel_restores_modes() {
  for cursor in [false, true] {
    for visual in [false, true] {
      let mut pane = Pane::default();
      if cursor {
        pane.enter_cursor();
      }
      if visual {
        pane.visual(SelectionKind::Character);
      }
      pane.notice("pending");
      pane.cancel_content();
      let expected = if cursor && visual { Mode::Cursor } else { Mode::Normal };
      assert_eq!(pane.mode(), expected);
      assert!(pane.selection.is_none());
      assert_eq!(pane.message, "");
      pane.cancel_content();
      assert_eq!(pane.mode(), Mode::Normal);
    }
  }
}

#[test]
fn reader_seed_copies_state_without_aliasing() -> Result<()> {
  let mut reader = Reader::default();
  reader.cursor.line = 5;
  reader.visual(SelectionKind::Line);
  reader.store('a', "original".into())?;
  reader.remember();
  let mut pane = Pane::new(reader.clone(), Tracking::default());
  pane.cancel();
  pane.store('a', "changed".into())?;
  pane.clear_history();
  assert_eq!(pane.register('a').map(|s| &**s), Some("changed"));
  assert_eq!(reader.registers.get(&'a').map(|s| &**s), Some("original"));
  assert_eq!(reader.mode(), Mode::Visual(SelectionKind::Line));
  assert_eq!(pane.mode(), Mode::Normal);
  assert_eq!(reader.jumps().count(), 1);
  assert_eq!(pane.jumps().count(), 0);
  assert_eq!(reader.pending_yank.as_ref().unwrap().text, "original");
  assert_eq!(pane.take_yank().unwrap().text, "changed");
  assert!(pane.take_yank().is_none());
  Ok(())
}

#[test]
fn reload_forgets_document_state() -> Result<()> {
  use crate::page::landing::Span;
  let now = Instant::now();
  let position = Position { line: 5, ..Position::default() };
  let mut fixture = Fixture::new();
  let mut reader = Reader::default();
  reader.remember();
  reader.visual(SelectionKind::Line);
  let mut pane = Pane::new(reader, Tracking::default());
  pane
    .bind(
      &mut fixture.layout,
      &mut fixture.text,
      fixture.frame,
      &mut fixture.source,
    )
    .land_at(position)?;
  pane.flash(position, Span::Glyph, now);
  pane.reload();
  assert_eq!(pane.mode(), Mode::Normal);
  assert_eq!(pane.jumps().count(), 0);
  assert_eq!(pane.target(), None);
  assert_eq!(pane.landing().deadline(), None);
  Ok(())
}

#[test]
fn tracking_copies_preserve_independent_feedback() -> Result<()> {
  use crate::page::landing::Span;
  use vpdf_model::document::Target;
  let now = Instant::now();
  let position = Position { line: 5, ..Position::default() };
  let mut fixture = Fixture::new();
  let mut pane = Pane::default();
  pane
    .bind(
      &mut fixture.layout,
      &mut fixture.text,
      fixture.frame,
      &mut fixture.source,
    )
    .land_at(position)?;
  pane.aim(Target::Page(PageId(0)));
  pane.animate_to(200.0, now);
  pane.flash(position, Span::Glyph, now);
  let copy = Pane::new((*pane).clone(), pane.tracking().clone());
  pane.cancel_scroll();
  pane.clear_target();
  pane.expire_flash(now + crate::page::landing::Landing::DURATION);
  assert_eq!(copy.target(), Some(position));
  assert_eq!(copy.animation().target(), Some(200.0));
  assert_eq!(copy.landing().position(), Some(position));
  assert_eq!(
    copy.landing().deadline(),
    Some(now + crate::page::landing::Landing::DURATION)
  );
  assert_eq!(copy.landing().aimed(), Some(&Target::Page(PageId(0))));
  assert_eq!(pane.target(), None);
  assert!(!pane.animation().active());
  assert_eq!(pane.landing().position(), None);
  Ok(())
}

#[test]
fn resize_preserves_landing_after_completed_reflow() -> Result<()> {
  for (text, fail) in [(false, false), (true, false), (true, true)] {
    let mut fixture = Fixture::new();
    fixture.frame.pane.text = text;
    let position = Position { line: 5, ..Position::default() };
    fixture.view().land_at(position)?;
    let before = fixture.view().landed_view(position);
    let deadline = fixture.landing.deadline();
    fixture.animation.start(200.0, Instant::now());
    fixture.source.fail = if fail { fixture.source.reads + 1 } else { 0 };
    let screen = Screen { columns: 8, ..fixture.frame.pane.screen };
    fixture.frame.pane.screen = screen;
    let result =
      fixture.view().resize(screen, &mut crate::scrollbar::Bar::default());
    assert_eq!(result.is_err(), fail);
    assert_eq!(fixture.layout.screen().columns, 8);
    assert_eq!(fixture.animation.active(), fail);
    assert_eq!(fixture.landing.deadline(), deadline);
    if fail {
      assert!(fixture.landing.matches(before));
    } else {
      assert!(fixture.view().matches_landing(position));
    }
  }
  Ok(())
}

#[test]
fn search_submission_preserves_failure_order() {
  for cursor in [false, true] {
    let mut fixture = Fixture::new();
    if cursor {
      fixture.reader.enter_cursor();
    }
    let mut search = crate::find::Search::default();
    search.begin_prompt(Location::default());
    fixture.reader.operator = Some(Operator { count: 1, register: 'a' });
    fixture.source.fail = 1;
    assert!(fixture.view().prepare_search(&mut search, "a", false).is_err());
    assert_eq!(fixture.source.reads, 1);
    assert!(fixture.reader.operator.is_none());
    assert!(search.prompt_origin().is_none());
    assert_eq!(search.take_previous().is_some(), !cursor);
    assert_eq!(fixture.reader.jumps().count(), 0);
  }
}

#[test]
fn search_abort_retains_failed_origin() {
  let mut fixture = Fixture::new();
  let mut search = crate::find::Search::default();
  search.begin_prompt(Location::default());
  search.cancel();
  fixture.source.fail = 1;
  assert!(fixture.view().abort_search(&mut search, true).is_err());
  assert!(search.prompt_origin().is_some());
  assert!(search.take_previous().is_some());
}

#[test]
fn search_submission_waits_for_scan() -> Result<()> {
  let mut fixture = Fixture::new();
  fixture.reader.enter_cursor();
  let mut search = crate::find::Search::default();
  let origin = Location::default();
  search.begin_prompt(origin);
  fixture.reader.cursor.line = 3;
  let submission = fixture.view().prepare_search(&mut search, "a", false)?;
  assert_eq!(submission.query().unwrap().text, "a");
  assert_eq!(submission.origin(), Some(origin.cursor));
  assert!(search.prompt_origin().is_none());
  assert_eq!(fixture.reader.jumps().count(), 0);
  search.begin(submission.query().unwrap().clone(), 1, origin.cursor);
  fixture.view().finish_search(&mut search, submission)?;
  assert_eq!(fixture.reader.jumps().count(), 1);
  assert!(fixture.landing.position().is_none());
  Ok(())
}

impl crate::source::Outline for Source {
  fn outline(&self) -> &[vpdf_model::document::Heading] {
    &self.headings
  }
}

fn named_headings() -> Vec<vpdf_model::document::Heading> {
  [15.0, 16.0, 110.0]
    .into_iter()
    .map(|y| vpdf_model::document::Heading {
      title: format!("Heading {y}"),
      depth: 0,
      target: vpdf_model::document::Target::Point {
        page: PageId(0),
        x: 10.0,
        y,
      },
    })
    .collect()
}

#[test]
fn named_section_prefers_aim_then_last_same_line() {
  let mut fixture = Fixture::new();
  fixture.source.headings = named_headings();
  let landed = fixture.view().landed_view(Position::default());
  fixture.landing.record(landed);
  fixture.landing.aim(fixture.source.headings[0].target.clone());
  fixture.source.fail = 1;
  assert_eq!(fixture.view().current_section(), Some(0));
  assert_eq!(fixture.source.reads, 0);
  fixture.landing = crate::page::landing::Landing::default();
  fixture.landing.record(landed);
  fixture.source.fail = 0;
  assert_eq!(fixture.view().current_section(), Some(1));
  assert_eq!(fixture.source.reads, 1);
}

#[test]
fn named_section_preserves_source_failure_fallback() {
  for text in [false, true] {
    let mut fixture = Fixture::new();
    fixture.source.headings = named_headings();
    fixture.frame.pane.text = text;
    fixture.source.fail = 1;
    assert_eq!(
      fixture.view().current_section(),
      if text { Some(2) } else { None }
    );
    assert_eq!(fixture.source.reads, usize::from(text));
  }
}

#[test]
fn outline_waits_for_focus_and_ignores_other_panels() {
  use crate::keys::PanelKind;
  use crate::panel::{Panel, Row};

  for kind in [PanelKind::Outline, PanelKind::Help] {
    let mut fixture = Fixture::new();
    fixture.source.headings = named_headings();
    fixture.reader.enter_cursor();
    fixture.reader.cursor.line = 5;
    let mut outline = Outline::default();
    let mut panel = Panel::new(
      kind,
      fixture
        .source
        .headings
        .iter()
        .map(|heading| Row::outline(heading, "1".into()))
        .collect(),
    );
    panel.focus(true);
    outline.track(&mut fixture.view(), Some(&mut panel));
    assert_eq!(outline.current(), Some(2));
    assert_eq!(panel.selected(), 0);
    panel.focus(false);
    outline.track(&mut fixture.view(), Some(&mut panel));
    assert_eq!(
      panel.selected(),
      if kind == PanelKind::Outline { 2 } else { 0 }
    );
    fixture.source.headings.clear();
    outline.track(&mut fixture.view(), Some(&mut panel));
    assert_eq!(outline.current(), None);
    fixture.source.headings = named_headings();
    fixture.reader.cursor.line = 0;
    outline.track(&mut fixture.view(), Some(&mut panel));
    assert_eq!(outline.current(), Some(1));
    assert_eq!(panel.selected(), usize::from(kind == PanelKind::Outline));
  }
}

#[test]
fn preparation_preserves_pending_operators() {
  use crate::keys::Action;
  use vpdf_reader::{motion::Motion, objects::Object};
  for pending in [false, true] {
    for (action, interrupts) in [
      (Action::Motion(Motion::Down), false),
      (Action::Object(Object::Word, true), false),
      (Action::Yank, false),
      (Action::YankLine, false),
      (Action::YankRaw, false),
      (Action::YankPath, false),
      (Action::Search(true), false),
      (Action::Cancel, false),
      (Action::Definition, true),
      (Action::Scroll(0.5), true),
      (Action::Zoom(1), true),
    ] {
      let mut reader = Reader::default();
      reader.operator = pending.then_some(Operator { count: 3, register: 'a' });
      let mut pane = Pane::new(reader, Tracking::default());
      assert_eq!(pane.prepare(action), pending && interrupts);
      assert_eq!(
        pane.operator.as_ref().map(|op| (op.count, op.register)),
        pending.then_some((3, 'a'))
      );
    }
  }
  let mut reader = Reader::default();
  reader.enter_cursor();
  reader.operator = Some(Operator { count: 2, register: 'a' });
  let mut pane = Pane::new(reader, Tracking::default());
  pane.animate_to(200.0, Instant::now());
  assert!(pane.prepare(Action::Zoom(1)));
  assert_eq!(pane.scroll, 200.0);
  assert!(!pane.animation().active());
}

#[test]
fn viewport_uses_page_geometry() -> Result<()> {
  use crate::find::Search;
  use crate::keys::Action;
  for (action, delta) in [
    (Action::Scroll(0.5), 90.0),
    (Action::Screen(1), 168.0),
    (Action::Rows(3), 72.0),
    (Action::Screen(-1), -168.0),
    (Action::Rows(-2), -48.0),
  ] {
    let mut fixture = Fixture::new();
    fixture.frame.pane.screen.rows = 30;
    fixture.frame.pane.screen.cell_height = 12;
    fixture.frame.visible.rows = 2;
    fixture.reader.scroll = 300.0;
    let input = crate::keys::Input { count: 2, ..input(action) };
    assert_eq!(
      fixture.view().viewport(input, &mut Search::default())?,
      ViewportEvent::Handled
    );
    assert_eq!(fixture.animation.target(), Some(300.0 + delta));
    assert_eq!(fixture.reader.scroll, 300.0);
  }
  let mut fixture = Fixture::new();
  for (action, expected) in [
    (Action::Crop, ViewportEvent::Crop),
    (Action::Definition, ViewportEvent::Unhandled),
    (Action::Zoom(1), ViewportEvent::Relayout),
    (Action::Fit, ViewportEvent::Relayout),
    (Action::FitPage, ViewportEvent::Relayout),
    (Action::Rotate, ViewportEvent::Relayout),
  ] {
    assert_eq!(
      fixture.view().viewport(input(action), &mut Search::default())?,
      expected
    );
  }
  assert_eq!(fixture.source.reads, 0);
  assert_eq!(fixture.layout.rotation(), 90);
  Ok(())
}

#[test]
fn viewport_failure_cancels_yank() {
  use crate::find::Search;
  use crate::keys::Action;
  use vpdf_search::{query::Query, search::Hit};
  let mut fixture = Fixture::new();
  fixture.reader.enter_cursor();
  fixture.reader.scroll = 50.0;
  fixture.reader.operator = Some(Operator { count: 1, register: 'a' });
  fixture.animation.start(200.0, Instant::now());
  fixture.source.fail = 1;
  let mut search = Search::default();
  let found = Position { line: 1, ..Position::default() };
  search.show(
    Query::default(),
    vec![Hit { first: found, last: found, excerpt: "a".into() }],
  );
  search.yank_after(Location::default(), Operator { count: 1, register: 'a' });
  assert!(
    fixture.view().viewport(input(Action::Rows(1)), &mut search).is_err()
  );
  assert_eq!(fixture.source.reads, 1);
  assert_eq!(fixture.reader.scroll, 200.0);
  assert_eq!(fixture.animation.target(), None);
  assert!(fixture.reader.operator.is_none());
  assert!(search.take_yank().is_none());
}

fn pointer_route(
  kind: crossterm::event::MouseEventKind,
  column: u16,
  row: u16,
) -> crate::pointer::Route {
  crate::pointer::Route {
    pane: 1,
    region: crate::pointer::Region::Page,
    focus: false,
    clicks: 1,
    pixel: None,
    mouse: crossterm::event::MouseEvent {
      kind,
      column,
      row,
      modifiers: crossterm::event::KeyModifiers::NONE,
    },
  }
}

#[test]
fn capture_avoids_text_and_clamps_corners() -> Result<()> {
  use crate::input::gesture::Gesture;
  use crossterm::event::{
    MouseButton::Left,
    MouseEventKind::{Down, Drag},
  };
  let mut fixture = Fixture::new();
  fixture.source.fail = 1;
  let mut gesture = Gesture::default();
  fixture.view().alt_press(&mut gesture, &pointer_route(Down(Left), 1, 1))?;
  assert!(gesture.sweep().is_some());
  fixture.view().sweep_to(
    &mut gesture,
    PageId(0),
    &pointer_route(Drag(Left), u16::MAX, u16::MAX),
  );
  assert_eq!(gesture.sweep().unwrap().corner(), Some((100.0, 1000.0)));
  assert_eq!(fixture.source.reads, 0);
  assert!(fixture.reader.selection.is_none());
  Ok(())
}

#[test]
fn failed_extensions_reset_previous_sweeps() {
  use crate::input::gesture::Gesture;
  use crossterm::event::{MouseButton::Left, MouseEventKind::Down};
  for text in [false, true] {
    let mut fixture = Fixture::new();
    fixture.frame.pane.text = text;
    fixture.source.fail = 1;
    let mut gesture = Gesture::default();
    gesture.begin_sweep(PageId(0), (10.0, 10.0));
    let press = pointer_route(Down(Left), u16::MAX, 1);
    assert!(fixture.view().alt_press(&mut gesture, &press).is_err());
    assert!(gesture.sweep().is_none());
    assert!(!gesture.dropped());
    assert_eq!(fixture.source.reads, 1);
    assert_eq!(fixture.reader.cursor, Position::default());
    assert!(fixture.reader.selection.is_none());
  }
}

#[test]
fn middle_drags_suppress_link_activation() -> Result<()> {
  use crate::input::grab::Grab;
  use crossterm::event::{
    MouseButton::Middle,
    MouseEventKind::{Down, Drag, Up},
  };
  for text in [false, true] {
    let mut fixture = Fixture::new();
    fixture.frame.pane.text = text;
    fixture.reader.scroll = 50.0;
    fixture.animation.start(200.0, Instant::now());
    let mut grab = Grab::default();
    assert!(
      !fixture.view().middle(&mut grab, &pointer_route(Down(Middle), 3, 4))?
    );
    assert_eq!(fixture.animation.target(), None);
    assert!(
      fixture.view().middle(&mut grab, &pointer_route(Up(Middle), 3, 4))?
    );
    for row in [4, 2] {
      fixture.view().middle(&mut grab, &pointer_route(Down(Middle), 3, 4))?;
      fixture.view().middle(&mut grab, &pointer_route(Drag(Middle), 3, row))?;
      assert!(
        !fixture
          .view()
          .middle(&mut grab, &pointer_route(Up(Middle), 3, row))?
      );
    }
    if text {
      assert_eq!(fixture.text.top().line, 2);
      assert_eq!(fixture.reader.scroll, 50.0);
    } else {
      assert_eq!(fixture.reader.scroll, 70.0);
      assert_eq!(fixture.source.reads, 0);
    }
  }
  Ok(())
}

impl crate::source::Markup for Source {
  fn links(
    &mut self,
    _: PageId,
  ) -> Result<Arc<Vec<vpdf_model::document::Link>>> {
    self.lookups.push("links");
    anyhow::ensure!(
      self.lookups.len() != self.fail_lookup,
      "Links unavailable"
    );
    Ok(Arc::new(self.links.clone()))
  }
  fn annotations(
    &mut self,
    _: PageId,
  ) -> Result<Arc<Vec<vpdf_model::document::Annotation>>> {
    self.lookups.push("notes");
    anyhow::ensure!(
      self.lookups.len() != self.fail_lookup,
      "Notes unavailable"
    );
    Ok(Arc::new(self.notes.clone()))
  }
}

#[test]
fn page_hits_preserve_precedence() -> Result<()> {
  use crate::pointer::Hit;
  use vpdf_model::document::{Annotation, Link, Target};
  for (link, fail, expected) in [
    (true, 2, Some(Hit::Link(PageId(0), 0))),
    (false, 0, Some(Hit::Note { page: PageId(0), index: 0, text: false })),
    (false, 3, Some(Hit::Note { page: PageId(0), index: 0, text: true })),
    (false, 1, None),
    (false, 2, None),
  ] {
    let mut fixture = Fixture::new();
    let bounds = fixture.source.text.lines[0].glyphs[0].bounds;
    fixture.source.fail = 1;
    fixture.source.fail_lookup = fail;
    if link {
      fixture.source.links.push(Link::new(bounds, Target::Page(PageId(0))));
    }
    fixture.source.notes.push(Annotation {
      kind: "Note".into(),
      areas: vec![bounds],
      contents: "note".into(),
      author: String::new(),
    });
    let result = fixture.view().hit(HitFrame::Pending, 1, 1, None);
    if let Some(expected) = expected {
      assert_eq!(result?, expected);
    } else {
      assert!(result.is_err());
    }
    let calls = if link {
      1
    } else if fail > 0 {
      fail
    } else {
      3
    };
    assert_eq!(fixture.source.lookups, ["links", "notes", "notes"][..calls]);
    assert_eq!(fixture.source.reads, 0);
  }
  Ok(())
}

#[test]
fn unmarked_hits_read_text() -> Result<()> {
  use crate::pointer::Hit;
  for (column, expected) in [(1, Hit::Text), (8, Hit::Nothing)] {
    let mut fixture = Fixture::new();
    assert_eq!(
      fixture.view().hit(HitFrame::Pending, column, 1, None)?,
      expected
    );
    assert_eq!(fixture.source.lookups, ["links", "notes"]);
    assert_eq!(fixture.source.reads, 1);
  }
  let mut fixture = Fixture::new();
  fixture.source.fail = 1;
  assert!(fixture.view().hit(HitFrame::Pending, 1, 1, None).is_err());
  assert_eq!(fixture.source.lookups, ["links", "notes"]);
  assert_eq!(fixture.source.reads, 1);
  Ok(())
}

#[test]
fn text_hits_follow_frames() -> Result<()> {
  use crate::pointer::Hit;
  use vpdf_model::document::{Link, Target};
  let mut fixture = Fixture::new();
  fixture.frame.pane.text = true;
  fixture.source.links = [0, 2]
    .into_iter()
    .map(|line| {
      Link::new(
        fixture.source.text.lines[line].glyphs[0].bounds,
        Target::Page(PageId(0)),
      )
    })
    .collect();
  let reader = Reader::default();
  let marks = paint::Marks {
    reader: &reader,
    matches: &[],
    flash: None,
    landing: None,
    cursorline: false,
    dim: false,
    hovered: &[],
  };
  fixture.view().text().draw(&mut Vec::new(), &marks)?;
  fixture.view().text().scroll(2)?;
  for (frame, index) in
    [(HitFrame::Drawn, 0), (HitFrame::Pending, 1), (HitFrame::Drawn, 0)]
  {
    assert_eq!(
      fixture.view().hit(frame, 0, 0, None)?,
      Hit::Link(PageId(0), index)
    );
  }
  fixture.view().text().draw(&mut Vec::new(), &marks)?;
  assert_eq!(
    fixture.view().hit(HitFrame::Drawn, 0, 0, None)?,
    Hit::Link(PageId(0), 1)
  );
  Ok(())
}

mod overlays;
