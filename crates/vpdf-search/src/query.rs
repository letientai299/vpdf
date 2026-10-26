/// A pattern with its direction and word matching.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Query {
  pub text: String,
  pub backward: bool,
  pub whole: bool,
  pub reference: Option<crate::reference::Lookup>,
}

impl Query {
  pub fn words(words: &[String]) -> Self {
    let alternatives = words
      .iter()
      .map(|word| fancy_regex::escape(word))
      .collect::<Vec<_>>()
      .join("|");
    Self { text: format!(r"\v(?:{alternatives})"), ..Self::default() }
  }
}
