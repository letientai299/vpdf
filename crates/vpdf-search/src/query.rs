/// A pattern with its direction and word matching.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Query {
  pub text: String,
  pub backward: bool,
  pub whole: bool,
  pub reference: Option<crate::reference::Lookup>,
}
