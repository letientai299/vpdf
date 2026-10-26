use super::Panel;
use crate::keys::PanelKind;
use vpdf_model::geometry::At;
use vpdf_model::scene::{Draw, Op};
use vpdf_model::style::Role;
use vpdf_model::text::clean;

impl Panel {
  /// What a panel without rows says, loading or done.
  pub(super) fn placeholder(&self) -> Option<String> {
    let loading = self.note == "loading";
    Some(match (self.kind, loading) {
      (PanelKind::Outline, true) => "Preparing contents…".into(),
      (PanelKind::Outline, false) => {
        "No headings. Use page jumps or search.".into()
      }
      (PanelKind::Figures, true) => "Finding figures…".into(),
      (PanelKind::Figures, false) => "No captioned figures or tables.".into(),
      (PanelKind::Results, true) => "Searching…".into(),
      (PanelKind::Results, false) if self.topic.is_empty() => {
        "No matches.".into()
      }
      (PanelKind::Results, false) => {
        format!("No matches for “{}”.", self.topic)
      }
      (PanelKind::Annotations, true) => "Reading annotations…".into(),
      (PanelKind::Annotations, false) => "No annotations in this PDF.".into(),
      (PanelKind::Jumps, _) => "No jumps yet.".into(),
      (PanelKind::Marks, _) => "No marks. Press m and a letter.".into(),
      _ => return None,
    })
  }

  /// Whether an empty panel still says something.
  pub fn explains_empty(&self) -> bool {
    self.placeholder().is_some()
  }

  /// One muted line under the header when there are no rows.
  pub(super) fn scene_empty<'a>(
    &self,
    scene: &mut impl Draw<'a>,
    at: At,
    inner: usize,
  ) {
    let Some(message) = self.placeholder().filter(|_| self.rows.is_empty())
    else {
      return;
    };
    crate::paint::move_to(scene, at, 1u16, 0u16);
    scene.push(Op::Style(Role::Muted));
    scene.text(&clean(&message, inner));
    scene.push(Op::Style(Role::Reset));
  }
}
