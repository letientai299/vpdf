use unicode_width::UnicodeWidthStr;
use vpdf_model::geometry::At;
use vpdf_model::scene::{Draw, Op};
use vpdf_model::style::{Color, Role};
use vpdf_model::text::clean;

pub(super) struct Header<'a> {
  pub title: &'a str,
  pub count: usize,
  pub context: &'a str,
  pub role: Role,
}

impl Header<'_> {
  pub fn scene<'a>(&self, scene: &mut impl Draw<'a>, at: At, columns: usize) {
    let count = format!(" {}", self.count);
    let title = clean(
      &format!(" {} ", self.title),
      columns.saturating_sub(count.width()),
    );
    let count = clean(&count, columns - title.width());
    let room = columns - title.width() - count.width();
    let context = clean(self.context, room);
    crate::paint::move_to(scene, at, 0u16, 0u16);
    scene.push(Op::Style(Role::Badge(Color::Cyan)));
    scene.text(&title);
    scene.push(Op::Style(self.role));
    scene.text(&count);
    scene.text(&context);
    scene.push(Op::Spaces(room - context.width()));
  }
}
