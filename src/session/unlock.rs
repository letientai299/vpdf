use super::native::Input;
use super::{PathBuf, Result, Screen, Session};

pub(super) enum Operation {
  Open(PathBuf),
  Reference(vpdf_model::reference::Reference),
  Request(super::Request),
}

pub(super) struct Unlock {
  operation: Operation,
  pub(super) prompt: vpdf_ui::prompt::Prompt,
  pub(super) retry: bool,
}

impl Session {
  pub(super) fn unlock_result(
    &mut self,
    result: Result<()>,
    operation: Option<Operation>,
  ) -> Result<()> {
    if matches!(self.frontend, crate::raster::Frontend::Native(_))
      && result
        .as_ref()
        .is_err_and(anyhow::Error::is::<super::PasswordRequired>)
      && let Some(operation) = operation
    {
      self.drop_pointer();
      self.unlock = Some(Unlock {
        operation,
        prompt: vpdf_ui::prompt::Prompt::new(
          vpdf_ui::prompt::PromptKind::Password,
        ),
        retry: false,
      });
      return Ok(());
    }
    result
  }

  pub(super) fn resume(
    &mut self,
    operation: Operation,
    screen: Screen,
  ) -> Result<()> {
    match operation {
      Operation::Open(path) => {
        self.resume(Operation::Reference(path.into()), screen)
      }
      Operation::Reference(reference) => {
        let tab = self.tabs.is_empty() || !self.focused().is_recent();
        let result = self.open_reference(&reference, tab, screen);
        self.unlock_result(result, Some(Operation::Reference(reference)))?;
        Ok(())
      }
      Operation::Request(request) => {
        let result = self.dispatch(Some(request.clone()), screen);
        self.unlock_result(result, Some(Operation::Request(request)))
      }
    }
  }

  pub(super) fn password_input(
    &mut self,
    input: &Input,
    screen: Screen,
  ) -> Result<bool> {
    use vpdf_ui::prompt::{History, PromptEvent};
    let Some(unlock) = &mut self.unlock else { return Ok(false) };
    unlock.prompt.set_bindings(
      self.settings.bindings().map(vpdf_ui::bindings::Context::Password),
    );
    let event = match input {
      Input::Key(key) => {
        if key.kind == vpdf_model::input::KeyEventKind::Release {
          return Ok(true);
        }
        unlock.prompt.feed(*key, &History::default())
      }
      Input::Paste(text) => unlock.prompt.paste(text),
      Input::Mouse { .. } | Input::Gesture { .. } | Input::History { .. } => {
        return Ok(true);
      }
      _ => return Ok(false),
    };
    match event {
      PromptEvent::Cancel => {
        self.unlock = None;
        self.pending_keys.clear();
      }
      PromptEvent::Submit => {
        let unlock = self.unlock.take().expect("Password prompt missing");
        self.secret = Some(unlock.prompt.into_text());
        let result = self.resume(unlock.operation, screen);
        self.secret = None;
        if let Some(unlock) = &mut self.unlock {
          unlock.retry = true;
        }
        if result.is_err() {
          self.pending_keys.clear();
        }
        result?;
      }
      _ => {}
    }
    Ok(true)
  }
}
