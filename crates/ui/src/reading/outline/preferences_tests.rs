use super::fold_tests::setup;
use super::*;

#[test]
fn preferences_wait_for_delivery() -> crate::Result<()> {
  let (mut outline, mut panel) = setup();
  assert!(outline.preferences().is_none());
  panel.choose(1);
  outline.fold(&mut panel, Fold::Close, 1);
  outline.fold(&mut panel, Fold::CollapseAll, 1);
  let encoded = serde_json::to_vec(outline.preferences().unwrap())?;
  let saved: Preferences = serde_json::from_slice(&encoded)?;
  let mut pending = Outline::default();
  pending.restore_preferences(Some(saved.clone()));
  pending.replace(&[]);
  let mut loading = Panel::new(PanelKind::Outline, vec![]);
  pending.fold(&mut loading, Fold::ExpandAll, 1);
  assert_eq!(pending.preferences(), Some(&saved));
  pending.sync(&outline.headings);
  assert!(pending.closed(0) && pending.closed(1) && pending.closed(4));
  assert_eq!(pending.preferences(), Some(&saved));
  pending.fold(&mut panel, Fold::ExpandAll, 1);
  assert!(pending.preferences().unwrap().newer(&saved));
  assert!(!pending.closed(0) && !pending.closed(1));
  Ok(())
}

#[test]
fn fingerprint_covers_outline() {
  let (mut outline, mut panel) = setup();
  outline.fold(&mut panel, Fold::CollapseAll, 1);
  let saved = outline.preferences().cloned();
  for change in 0..6 {
    let mut headings = outline.headings.clone();
    match change {
      0 => headings[1].depth += 1,
      1 => headings[1].title.push('!'),
      2 => headings[1].target = Target::Page(PageId(1)),
      3 => {
        headings[1].target = Target::Point { page: PageId(0), x: 0.0, y: 1.0 }
      }
      4 => headings.swap(0, 1),
      _ => {
        headings.pop();
      }
    }
    let mut restored = Outline::new(&headings);
    restored.restore_preferences(saved.clone());
    assert!(restored.closed.iter().all(|&closed| !closed), "{change}");
  }
  let mut restored = Outline::new(&outline.headings);
  restored.restore_preferences(saved);
  assert!(restored.closed(0));
}

#[test]
fn previews_preserve_revision() {
  let (mut outline, mut panel) = setup();
  outline.fold(&mut panel, Fold::CollapseAll, 1);
  let saved = outline.preferences().cloned();
  panel.begin_search(false);
  outline.preview_search(&mut panel, "iv");
  outline.abort_search(&mut panel);
  assert_eq!(outline.preferences(), saved.as_ref());
  panel.begin_search(false);
  assert!(!outline.finish_search(&mut panel, "absent"));
  assert_eq!(outline.preferences(), saved.as_ref());
  panel.begin_search(false);
  assert!(outline.finish_search(&mut panel, "iv"));
  assert!(outline.preferences().unwrap().newer(saved.as_ref().unwrap()));
  outline.toggle_auto(&mut panel);
  let saved = outline.preferences().cloned();
  let mut restored = Outline::new(&outline.headings);
  restored.restore_preferences(saved.clone());
  assert!(restored.auto);
  restored.current = Some(2);
  restored.expand_current();
  assert_eq!(restored.preferences(), saved.as_ref());
  assert!(!restored.closed(0) && !restored.closed(1));
  assert!(restored.closed(4));
}

#[test]
fn pending_modes_preserve_folds() {
  let (mut outline, mut panel) = setup();
  outline.fold(&mut panel, Fold::CollapseAll, 1);
  let saved = outline.preferences().cloned();
  let mut pending = Outline::default();
  let mut loading = Panel::new(PanelKind::Outline, vec![]);
  pending.restore_preferences(saved.clone());
  pending.toggle_auto(&mut loading);
  let automatic = pending.preferences().unwrap().clone();
  assert!(automatic.newer(saved.as_ref().unwrap()));
  assert_eq!(
    serde_json::to_value(&automatic).unwrap()["closed"],
    serde_json::to_value(saved.as_ref().unwrap()).unwrap()["closed"]
  );
  pending.toggle_auto(&mut loading);
  pending.sync(&outline.headings);
  assert!(!pending.auto);
  assert!(pending.closed(0) && pending.closed(1));
  let mut fresh = Outline::default();
  fresh.toggle_auto(&mut loading);
  let mut restored = Outline::new(&outline.headings);
  restored.restore_preferences(fresh.preferences().cloned());
  assert!(restored.auto && restored.closed(0));
}

#[test]
fn pending_folds_disable_auto() {
  for operation in [Fold::Close, Fold::Open, Fold::ExpandAll, Fold::CollapseAll]
  {
    let (mut outline, mut panel) = setup();
    outline.fold(&mut panel, Fold::CollapseAll, 1);
    let saved = outline.preferences().cloned();
    let mut pending = Outline::default();
    let mut loading = Panel::new(PanelKind::Outline, vec![]);
    pending.restore_preferences(saved);
    pending.toggle_auto(&mut loading);
    let automatic = pending.preferences().cloned().unwrap();
    pending.fold(&mut loading, operation, 1);
    assert!(!loading.auto_expand(), "{operation:?}");
    assert!(pending.preferences().unwrap().newer(&automatic));
    pending.sync(&outline.headings);
    pending.project(&mut panel);
    assert!(!panel.auto_expand());
    assert!(pending.closed(0) && pending.closed(1) && pending.closed(4));
  }
}
