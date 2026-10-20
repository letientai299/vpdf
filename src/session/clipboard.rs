use super::{Duration, Instant, Session, Write, terminal};

pub(super) struct ImageWrite {
  pane: u32,
  capture: vpdf_ui::region::CaptureId,
  state: ImageState,
}

enum ImageState {
  Waiting(Instant),
  TimedOut,
}

impl Session {
  fn image_message(&mut self, write: &ImageWrite, message: String) {
    if let Some(app) = self
      .tabs
      .iter_mut()
      .flat_map(|tab| &mut tab.panes)
      .find(|app| app.id == write.pane)
    {
      app.region_status(write.capture, message);
    }
  }

  pub(super) fn image_status(&mut self, body: &str) -> bool {
    let Some(result) = vpdf_terminal::clipboard::image_status(body) else {
      return false;
    };
    let Some(write) = self.image_write.take() else { return false };
    if matches!(write.state, ImageState::Waiting(_)) {
      let message = result
        .map_or_else(|error| error.to_string(), |()| "Image copied".into());
      self.image_message(&write, message);
    }
    true
  }

  pub(super) fn image_wait(&self) -> Option<Duration> {
    match &self.image_write {
      Some(ImageWrite { state: ImageState::Waiting(until), .. }) => {
        Some(until.saturating_duration_since(Instant::now()))
      }
      _ => None,
    }
  }

  pub(super) fn image_timeout(&mut self, now: Instant) -> bool {
    let Some(mut write) = self.image_write.take() else { return false };
    let expired =
      matches!(write.state, ImageState::Waiting(until) if now >= until);
    if expired {
      write.state = ImageState::TimedOut;
      self.image_message(&write, "Image clipboard timed out".into());
    }
    self.image_write = Some(write);
    expired
  }

  pub(super) fn image_updates(&mut self, out: &mut impl Write) -> bool {
    let mut dirty = false;
    let terminal = matches!(self.frontend, crate::raster::Frontend::Terminal);
    for app in self.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
      let result = app.region_updates().and_then(|changed| {
        if changed && terminal {
          app.clipboard(out)?;
        }
        Ok(changed)
      });
      match result {
        Ok(changed) => dirty |= changed,
        Err(error) => {
          app.error = Some(error.to_string());
          dirty = true;
        }
      }
    }
    let transfers: Vec<_> = self
      .tabs
      .iter_mut()
      .flat_map(|tab| &mut tab.panes)
      .filter_map(|app| app.region_send.take().map(|send| (app.id, send)))
      .collect();
    for (pane, transfer) in transfers {
      dirty = true;
      let write = ImageWrite {
        pane,
        capture: transfer.id,
        state: ImageState::Waiting(Instant::now() + Duration::from_secs(10)),
      };
      if self.image_write.is_some() {
        self.image_message(
          &write,
          "Image clipboard awaiting previous status".into(),
        );
        continue;
      }
      match vpdf_terminal::caps::get().and_then(|caps| caps.clipboard) {
        Some(vpdf_terminal::caps::Support::Answered) => {}
        Some(_) => {
          self.image_message(&write, "Image clipboard unsupported".into());
          continue;
        }
        None => {
          self
            .image_message(&write, "Image clipboard support unanswered".into());
          continue;
        }
      }
      self.image_write = Some(write);
      let result = vpdf_terminal::clipboard::image_remote(
        out,
        &transfer.png,
        terminal::tmux(),
      )
      .and_then(|()| out.flush().map_err(Into::into));
      if let Err(error) = result {
        let mut write = self.image_write.take().expect("reserved image write");
        write.state = ImageState::TimedOut;
        self.image_message(&write, error.to_string());
        self.image_write = Some(write);
      }
    }
    dirty
  }
}

#[cfg(test)]
mod tests {
  use super::super::run::{act, idle, input};
  use super::super::{PathBuf, Result, Screen};
  use super::*;

  #[test]
  fn image_timeout_retains_transfer() {
    let mut session = Session::new(None);
    let capture = crate::region::next_capture();
    let now = Instant::now();
    session.image_write = Some(ImageWrite {
      pane: 999,
      capture,
      state: ImageState::Waiting(now + Duration::from_secs(10)),
    });
    assert!(!session.image_timeout(now));
    assert!(session.image_timeout(now + Duration::from_secs(10)));
    assert!(!session.image_timeout(now + Duration::from_secs(20)));
    let original = session.image_write.as_ref().unwrap();
    assert_eq!(original.capture, capture);
    assert!(matches!(original.state, ImageState::TimedOut));
    assert!(!session.image_status("type=read:status=DONE"));
    assert!(session.image_write.is_some());
    assert!(session.image_status("type=write:status=DONE"));
    assert!(session.image_write.is_none());
    assert!(!session.image_status("type=write:status=DONE"));
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn image_status_preserves_bar() -> Result<()> {
    use crate::region::Transfer;
    use std::collections::VecDeque;
    let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    let screen = Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.open(&root.join("paper.pdf"), true, screen)?;
    session.app().move_figure(true, 1)?;
    session.app().open_region()?;
    let old = session.app().overlays.region().unwrap().snapshot().clone();
    let pane = session.app().id;
    session.image_write = Some(ImageWrite {
      pane,
      capture: old.id,
      state: ImageState::Waiting(Instant::now() + Duration::from_millis(500)),
    });
    let now = Instant::now();
    assert!(idle(&mut session, now, now, false) <= Duration::from_millis(500));
    session.app().select_region(old.page, old.bounds)?;
    let new = session.app().overlays.region().unwrap().snapshot().clone();
    assert_ne!(old.id, new.id);
    session.app().region_status(new.id, "New operation".into());
    let mut events = VecDeque::new();
    vpdf_terminal::decode::Parser::default()
      .feed(b"\x1b]5522;type=write:status=DONE\x1b\\", &mut events);
    let event = act(events.pop_front().unwrap()).unwrap();
    assert!(!event.typed());
    input(&mut session, event, &mut Vec::new())?;
    assert!(session.image_write.is_none());
    assert_eq!(
      session.app().overlays.region().unwrap().status(),
      Some("New operation")
    );
    assert_eq!(session.app().reader.message, "New operation");
    session.image_write =
      Some(ImageWrite { pane, capture: old.id, state: ImageState::TimedOut });
    session.app().region_send = Some(Transfer { id: new.id, png: vec![1] });
    let mut output = Vec::new();
    assert!(session.image_updates(&mut output));
    assert_eq!(output, [] as [u8; 0]);
    assert_eq!(session.image_write.as_ref().unwrap().capture, old.id);
    assert_eq!(
      session.app().reader.message,
      "Image clipboard awaiting previous status"
    );
    assert!(session.image_status("type=write:status=EPERM"));
    assert!(session.image_write.is_none());
    Ok(())
  }
}
