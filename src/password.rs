//! The password line, read before any pane opens.
use vpdf_ui::prompt::{History, Prompt, PromptEvent, PromptKind};

pub fn password() -> crate::Result<String> {
  use std::io::{self, Write};
  let mut prompt = Prompt::new(PromptKind::Password);
  let mut out = io::stdout();
  loop {
    // Rows and columns only: a terminal that lost its pixel metrics
    // still takes a password.
    let (columns, rows) = crossterm::terminal::size()?;
    vpdf_terminal::terminal::line(
      &mut out,
      vpdf_model::geometry::At::default(),
      rows.max(1) - 1,
      &prompt.display().0,
      columns,
    )?;
    out.flush()?;
    let Some(event) = vpdf_terminal::input::next(None)? else { continue };
    match password_event(&mut prompt, event) {
      PromptEvent::Submit => return Ok(prompt.into_text()),
      PromptEvent::Cancel => anyhow::bail!("Password cancelled"),
      _ => {}
    }
  }
}

/// Edits the password line; a resize goes on to the event loop, which
/// lays out the panes again once the prompt closes.
fn password_event(
  prompt: &mut Prompt,
  event: vpdf_terminal::decode::Event,
) -> PromptEvent {
  use vpdf_model::input::KeyEventKind;
  use vpdf_terminal::decode::Event;
  match event {
    Event::Key(key) if key.kind != KeyEventKind::Release => {
      prompt.feed(key, &History::default())
    }
    Event::Paste(text) => prompt.paste(&text),
    Event::Resize(report) => {
      vpdf_terminal::terminal::defer_resize(report);
      PromptEvent::Pending
    }
    _ => PromptEvent::Pending,
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};

  #[test]
  fn password_prompt_defers_resizes() {
    use vpdf_terminal::decode::Event;
    let mut prompt = Prompt::new(PromptKind::Password);
    let event = password_event(&mut prompt, Event::Resize(None));
    assert_eq!(event, PromptEvent::Pending);
    assert!(vpdf_terminal::terminal::resize_deferred(), "passed on");
    assert!(!vpdf_terminal::terminal::resize_deferred());
    let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(password_event(&mut prompt, enter), PromptEvent::Submit);
  }
}
