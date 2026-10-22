use vpdf_model::input as core;
use vpdf_terminal::decode as wire;

pub(crate) fn event(event: wire::Event) -> Option<core::Event> {
  Some(match event {
    wire::Event::Key(value) => core::Event::Key(value),
    wire::Event::Mouse(value) => core::Event::Mouse(value),
    wire::Event::Paste(text) => core::Event::Paste(text),
    _ => return None,
  })
}
