use crate::text::PageId;

pub(crate) const PAGES: usize = 10;

pub(crate) fn nearby(
  count: usize,
  anchor: PageId,
  forward: bool,
) -> Vec<PageId> {
  let mut pages = Vec::with_capacity(PAGES);
  for distance in 0..PAGES {
    let next = anchor.0.checked_add(distance).filter(|page| *page < count);
    let previous = anchor.0.checked_sub(distance);
    let sides = if forward { [next, previous] } else { [previous, next] };
    for page in sides.into_iter().flatten().map(PageId) {
      if !pages.contains(&page) {
        pages.push(page);
      }
      if pages.len() == PAGES {
        return pages;
      }
    }
  }
  pages
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn bounded_neighborhood() {
    for count in [1, 2, 10, 244, 3329] {
      for anchor in [0, count / 2, count - 1] {
        for forward in [false, true] {
          let pages = nearby(count, PageId(anchor), forward);
          assert_eq!(pages.len(), count.min(PAGES));
          assert_eq!(pages[0], PageId(anchor));
          assert!(pages.iter().all(|page| page.0 < count));
          let mut unique = pages.clone();
          unique.sort_unstable();
          unique.dedup();
          assert_eq!(unique.len(), pages.len());
        }
      }
    }
    assert_eq!(
      &nearby(100, PageId(50), true)[..3],
      &[PageId(50), PageId(51), PageId(49)]
    );
    assert_eq!(
      &nearby(100, PageId(50), false)[..3],
      &[PageId(50), PageId(49), PageId(51)]
    );
  }
}
