pub fn metadata_lines(metadata: &[(String, String)]) -> Vec<String> {
  let fields = metadata.iter().map(|(k, v)| format!("{k}: {v}"));
  std::iter::once("# Metadata".to_owned()).chain(fields).collect()
}

/// `:info`'s file facts, shared with recent document cards; `place`
/// names where the file lives.
pub fn document_lines(place: String, bytes: u64, pages: usize) -> Vec<String> {
  vec![
    "# Document".into(),
    place,
    format!("Size: {:.1} MiB", bytes as f64 / 1_048_576.0),
    format!("Pages: {pages}"),
  ]
}

pub fn help(installed: impl Fn(&str) -> bool) -> Vec<String> {
  let keys = include_str!("../help.txt").lines().map(str::to_owned);
  let commands = crate::commands::commands(installed).map(|spec| {
    let usage = if spec.usage.is_empty() { "" } else { " " };
    format!(":{}{usage}{}: {}", spec.name, spec.usage, spec.help)
  });
  keys.chain([String::new(), "# Commands".into()]).chain(commands).collect()
}
