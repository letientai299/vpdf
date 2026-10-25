use vpdf_model::document::Heading;

#[derive(Debug)]
pub struct Node {
  pub depth: usize,
  pub parent: Option<usize>,
  pub end: usize,
  pub descendants: usize,
}

#[derive(Debug, Default)]
pub struct Tree {
  nodes: Vec<Node>,
}

impl Tree {
  pub fn new(headings: &[Heading]) -> Self {
    let mut nodes: Vec<Node> = Vec::with_capacity(headings.len());
    let mut parents: Vec<usize> = Vec::new();
    for (source, heading) in headings.iter().enumerate() {
      while let Some(&parent) = parents.last()
        && nodes[parent].depth >= heading.depth
      {
        nodes[parent].end = source;
        nodes[parent].descendants = source - parent - 1;
        parents.pop();
      }
      nodes.push(Node {
        depth: heading.depth,
        parent: parents.last().copied(),
        end: headings.len(),
        descendants: headings.len() - source - 1,
      });
      parents.push(source);
    }
    Self { nodes }
  }

  pub fn nodes(&self) -> &[Node] {
    &self.nodes
  }

  pub fn project(&self, mut closed: impl FnMut(usize) -> bool) -> Projection {
    let mut sources = Vec::new();
    let mut positions = vec![None; self.nodes.len()];
    let mut hidden_until = 0;
    for (source, node) in self.nodes.iter().enumerate() {
      if source < hidden_until {
        continue;
      }
      positions[source] = Some(sources.len());
      sources.push(source);
      if closed(source) {
        hidden_until = node.end;
      }
    }
    Projection { sources, positions }
  }

  pub fn visible_ancestor(
    &self,
    mut source: usize,
    projection: &Projection,
  ) -> Option<usize> {
    loop {
      let node = self.nodes.get(source)?;
      if projection.position(source).is_some() {
        return Some(source);
      }
      source = node.parent?;
    }
  }
}

#[derive(Debug)]
pub struct Projection {
  sources: Vec<usize>,
  positions: Vec<Option<usize>>,
}

impl Projection {
  pub fn all(count: usize) -> Self {
    Self {
      sources: (0..count).collect(),
      positions: (0..count).map(Some).collect(),
    }
  }

  pub fn sources(&self) -> &[usize] {
    &self.sources
  }

  pub fn position(&self, source: usize) -> Option<usize> {
    self.positions.get(source).copied().flatten()
  }

  pub fn source(&self, position: usize) -> Option<usize> {
    self.sources.get(position).copied()
  }

  pub(crate) fn source_count(&self) -> usize {
    self.positions.len()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_model::document::Target;
  use vpdf_model::text::PageId;

  fn headings(depths: &[usize]) -> Vec<Heading> {
    depths
      .iter()
      .map(|&depth| Heading {
        depth,
        title: "  Duplicate".into(),
        target: Target::Page(PageId(0)),
      })
      .collect()
  }

  #[test]
  fn tree_shapes() {
    type Case = (&'static [usize], &'static [(Option<usize>, usize)]);
    let cases: &[Case] = &[
      (&[], &[]),
      (&[0], &[(None, 1)]),
      (&[0, 0, 0], &[(None, 1), (None, 2), (None, 3)]),
      (&[0, 1, 1, 0], &[(None, 3), (Some(0), 2), (Some(0), 3), (None, 4)]),
      (
        &[2, 5, 8, 4, 1],
        &[(None, 4), (Some(0), 3), (Some(1), 3), (Some(0), 4), (None, 5)],
      ),
    ];
    for &(depths, expected) in cases {
      let tree = Tree::new(&headings(depths));
      let actual: Vec<_> =
        tree.nodes().iter().map(|node| (node.parent, node.end)).collect();
      assert_eq!(actual, expected, "{depths:?}");
      for (source, node) in tree.nodes().iter().enumerate() {
        assert_eq!(node.depth, depths[source]);
        assert_eq!(node.descendants, node.end - source - 1);
      }
      let projection = tree.project(|_| false);
      assert_eq!(projection.sources(), (0..depths.len()).collect::<Vec<_>>());
    }
  }

  #[test]
  fn nested_projection() {
    let tree = Tree::new(&headings(&[0, 1, 2, 1, 0, 3, 5]));
    for (closed, visible) in [
      (&[][..], &[0, 1, 2, 3, 4, 5, 6][..]),
      (&[1][..], &[0, 1, 3, 4, 5, 6][..]),
      (&[0, 1][..], &[0, 4, 5, 6][..]),
      (&[0, 4][..], &[0, 4][..]),
      (&[2, 3, 6][..], &[0, 1, 2, 3, 4, 5, 6][..]),
    ] {
      let projection = tree.project(|source| closed.contains(&source));
      assert_eq!(projection.sources(), visible);
      for source in 0..tree.nodes().len() {
        assert_eq!(
          projection.position(source),
          visible.iter().position(|&index| index == source)
        );
        let ancestor = tree.visible_ancestor(source, &projection).unwrap();
        assert!(visible.contains(&ancestor));
        assert!(ancestor <= source && source < tree.nodes()[ancestor].end);
      }
      assert_eq!(projection.source(visible.len()), None);
      assert_eq!(projection.position(tree.nodes().len()), None);
      assert_eq!(tree.visible_ancestor(tree.nodes().len(), &projection), None);
    }
    let collapsed = tree.project(|source| [0, 4].contains(&source));
    assert_eq!(tree.visible_ancestor(2, &collapsed), Some(0));
    assert_eq!(tree.visible_ancestor(6, &collapsed), Some(4));
    assert_eq!(tree.nodes()[0].descendants, 3);
    assert_eq!(tree.nodes()[4].descendants, 2);
  }

  #[test]
  fn deep_tree() {
    let depths: Vec<_> = (0..10_000).collect();
    let tree = Tree::new(&headings(&depths));
    assert_eq!(tree.nodes()[9_999].parent, Some(9_998));
    assert_eq!(tree.nodes()[0].descendants, 9_999);
    let projection = tree.project(|source| source == 0);
    assert_eq!(projection.sources(), [0]);
    assert_eq!(tree.visible_ancestor(9_999, &projection), Some(0));
  }
}
