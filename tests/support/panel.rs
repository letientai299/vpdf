use vpdf_ui::keys::PanelKind;
use vpdf_ui::panel::{Panel, Row};

pub fn panel() -> Panel {
  Panel::new(
    PanelKind::Help,
    ["# 日本語", "j: down", "k: up"].map(Row::parse).into(),
  )
}
