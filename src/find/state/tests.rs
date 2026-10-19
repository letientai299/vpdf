use super::*;
use crate::text::PageId;

fn at(page: usize) -> Position {
  Position { page: PageId(page), ..Position::default() }
}

fn hit(page: usize) -> Hit {
  Hit { first: at(page), last: at(page), excerpt: String::new() }
}

fn event(generation: u64, pages: &[usize], done: bool) -> SearchEvent {
  let hits = pages.iter().map(|&page| hit(page)).collect();
  SearchEvent { generation, hits, done, error: None }
}

fn running(backward: bool, origin: usize) -> Search {
  let mut search = Search::default();
  let query = Query { text: "a".into(), backward, ..Query::default() };
  search.begin(query, 7, at(origin));
  search
}

fn pages(search: &Search) -> Vec<usize> {
  search.hits().iter().map(|hit| hit.first.page.0).collect()
}

#[test]
fn stale_generations_are_ignored() {
  let mut search = running(false, 0);
  assert_eq!(search.accept(event(6, &[1], true)), None);
  assert!(search.hits().is_empty());
  search.cancel();
  assert_eq!(search.accept(event(7, &[1], true)), None);
}

#[test]
fn page_events_merge_in_order() {
  let mut search = running(false, 0);
  for batch in [&[5, 6][..], &[1], &[3]] {
    let update = search.accept(event(7, batch, false));
    assert_eq!(update, Some(Update::Progress { done: false }));
  }
  assert_eq!(pages(&search), [1, 3, 5, 6]);
}

#[test]
fn failure_completes_and_drops_the_yank() {
  let mut search = running(false, 0);
  search.yank_after(Location::default(), Operator { count: 1, register: '"' });
  let mut failed = event(7, &[2], false);
  failed.error = Some("stopped".into());
  assert_eq!(search.accept(failed), Some(Update::Failed("stopped".into())));
  assert_eq!(pages(&search), [2]);
  assert!(matches!(search.phase(), Phase::Complete));
  assert!(search.take_yank().is_none());
}

#[test]
fn preview_follows_direction_and_wraps_when_done() {
  let cases = [
    (false, 4, false, None),
    (false, 4, true, Some(1)),
    (false, 2, false, Some(3)),
    (true, 2, false, Some(1)),
    (true, 0, false, None),
    (true, 0, true, Some(3)),
  ];
  for (backward, origin, done, want) in cases {
    let mut search = running(backward, origin);
    search.accept(event(7, &[1, 3], done));
    let got = search.preview(done).map(|position| position.page.0);
    assert_eq!(got, want, "backward={backward} origin={origin} done={done}");
  }
}

#[test]
fn preview_stops_after_jumping() {
  let mut search = running(false, 0);
  search.accept(event(7, &[1], false));
  search.stop_jumping();
  assert_eq!(search.preview(false), None);
  search.define(at(0));
  assert_eq!(search.preview(true), None);
  assert!(search.complete(), "a definition scan");
  assert!(!search.complete());
}

#[test]
fn yank_waits_for_completion() {
  let mut search = running(false, 0);
  let origin = Location { cursor: at(2), ..Location::default() };
  search.yank_after(origin, Operator { count: 2, register: 'a' });
  search.accept(event(7, &[1, 3, 4], true));
  assert!(search.take_yank().is_none(), "still running");
  search.complete();
  let (_, operator, target) = search.take_yank().unwrap();
  assert_eq!((operator.register, target), ('a', at(4)));
  assert!(search.take_yank().is_none());
}

#[test]
fn unreached_yank_ends() {
  let mut search = running(false, 0);
  search.yank_after(Location::default(), Operator { count: 1, register: 'a' });
  search.accept(event(7, &[], true));
  search.complete();
  assert!(search.take_yank().is_none(), "no hit to reach");
  search.accept(event(7, &[1], true));
  assert!(search.take_yank().is_none(), "the yank ended");
}

#[test]
fn cancel_hides_and_ignores_the_scan() {
  let mut search = running(false, 0);
  search.yank_after(Location::default(), Operator { count: 1, register: 'a' });
  search.cancel();
  assert!(search.hidden() && matches!(search.phase(), Phase::Idle));
  assert_eq!(search.generation(), 8);
  search.complete();
  assert!(search.take_yank().is_none());
}

#[test]
fn previous_search_survives_a_prompt() {
  let mut search = running(false, 0);
  search.set_hidden(true);
  search.remember_previous();
  let other = Query { text: "b".into(), ..Query::default() };
  search.begin(other, 8, at(0));
  let (query, hidden) = search.take_previous().unwrap();
  assert_eq!((query.text.as_str(), hidden), ("a", true));
  assert!(search.take_previous().is_none());
  search.remember_previous();
  search.forget_previous();
  assert!(search.take_previous().is_none());
}

#[test]
fn show_installs_finished_hits() {
  let mut search = Search::default();
  search.set_hidden(true);
  search.show(Query::default(), vec![hit(2)]);
  assert_eq!(pages(&search), [2]);
  assert_eq!(search.generation(), 1);
  assert!(!search.hidden() && matches!(search.phase(), Phase::Complete));
}
