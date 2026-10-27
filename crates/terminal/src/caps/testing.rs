use super::{Caps, Cell, Host, Passthrough, Support, Tmux};

/// Caps with no answer.
pub(crate) fn bare(host: Host) -> Caps {
  Caps {
    host,
    version: None,
    keyboard: None,
    background: None,
    sync: Support::Missing,
    pixels: Support::Missing,
    resize: Support::Missing,
    pointer: Support::Missing,
    drop: Support::Missing,
    clipboard: None,
    cell: None,
    attributes: None,
  }
}

pub(crate) fn tmux_info(client: Option<&str>) -> Tmux {
  Tmux {
    version: vec![3, 8],
    client: client.map(str::to_owned),
    cell: Some(Cell { width: 7, height: 13 }),
    passthrough: Some(Passthrough::On),
    extended_keys: true,
    focus_events: true,
    origin: (0, 0),
  }
}
