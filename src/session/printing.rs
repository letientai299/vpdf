use super::{Duration, Result, Session};
use crate::programs::Dialog;
use std::task::Poll;

pub(super) struct Job {
  pane: u32,
  dialog: Dialog,
}

impl Job {
  fn start(pane: u32, path: &std::path::Path) -> Result<Self> {
    let args = ["-l", "JavaScript", "-e", include_str!("print.js"), "--"];
    Ok(Self { pane, dialog: Dialog::osascript(&args, path)? })
  }

  fn poll(&mut self) -> Result<Poll<()>> {
    let Poll::Ready(output) = self.dialog.poll("Print dialog failed")? else {
      return Ok(Poll::Pending);
    };
    match output.trim() {
      "submitted" | "cancelled" => Ok(Poll::Ready(())),
      "encrypted" => anyhow::bail!("Encrypted PDF printing unavailable"),
      "unavailable" => anyhow::bail!("Printing requires a desktop"),
      "denied" => anyhow::bail!("PDF printing permission denied"),
      "open-failed" => anyhow::bail!("Cannot open PDF for printing"),
      _ => anyhow::bail!("Print dialog failed"),
    }
  }
}

impl Session {
  pub(super) fn print(&mut self) -> Result<()> {
    anyhow::ensure!(cfg!(target_os = "macos"), "Printing requires macOS");
    anyhow::ensure!(self.printing.is_none(), "Print dialog already open");
    let app = self.app()?;
    anyhow::ensure!(app.manual.is_none(), "Help printing unavailable");
    let stamp = crate::watch::stamp(&app.path);
    anyhow::ensure!(stamp.is_some(), "Cannot open PDF for printing");
    anyhow::ensure!(stamp == app.stamp, "PDF changed; reload before printing");
    let path = std::fs::canonicalize(&app.path)?;
    self.printing = Some(Job::start(app.id, &path)?);
    Ok(())
  }

  pub(super) fn print_update(&mut self) -> bool {
    let Some(job) = &mut self.printing else { return false };
    let result = job.poll();
    if matches!(result, Ok(Poll::Pending)) {
      return false;
    }
    let pane = job.pane;
    self.printing = None;
    if let Err(error) = result
      && let Some(app) = self.pane_mut(pane)
    {
      app.error = Some(error.to_string());
    }
    true
  }

  pub(super) fn print_wait(&self, wait: Duration) -> Duration {
    if self.printing.is_some() {
      wait.min(Duration::from_millis(50))
    } else {
      wait
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::Path;
  use std::process::{Command, Stdio};
  use std::time::Instant;

  fn complete(job: &mut Job) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
      match job.poll()? {
        Poll::Ready(()) => return Ok(()),
        Poll::Pending => {
          anyhow::ensure!(Instant::now() < deadline, "Print job timed out");
          std::thread::sleep(Duration::from_millis(1));
        }
      }
    }
  }

  fn response(pane: u32, output: &str) -> Result<Job> {
    let dialog = Dialog::exited("/usr/bin/printf", &["%s", output])?;
    Ok(Job { pane, dialog })
  }

  fn opened(root: &Path) -> Result<Session> {
    let path = root.join("print.pdf");
    crate::fixtures::pdf(&path, &["first", "second"], "")?;
    let mut session = Session::new(None);
    session.open(&path, true, crate::screen::cells(120, 40))?;
    Ok(session)
  }

  #[test]
  fn print_responses() -> Result<()> {
    for (output, expected) in [
      ("submitted\n", None),
      ("cancelled\n", None),
      ("encrypted\n", Some("Encrypted PDF printing unavailable")),
      ("unavailable\n", Some("Printing requires a desktop")),
      ("denied\n", Some("PDF printing permission denied")),
      ("open-failed\n", Some("Cannot open PDF for printing")),
      ("failed\n", Some("Print dialog failed")),
      ("", Some("Print dialog failed")),
      ("unexpected", Some("Print dialog failed")),
    ] {
      let mut job = response(0, output)?;
      assert_eq!(
        complete(&mut job).err().map(|error| error.to_string()).as_deref(),
        expected,
        "{output:?}"
      );
    }
    let dialog = Dialog::exited("/usr/bin/false", &[])?;
    assert_eq!(
      complete(&mut Job { pane: 0, dialog }).unwrap_err().to_string(),
      "Print dialog failed"
    );
    Ok(())
  }

  #[test]
  fn completion_tracks_origin() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut session = opened(root.path())?;
    let pane = session.app().unwrap().id;
    session.app().unwrap().command("vsplit")?;
    session.settle()?;
    assert_ne!(session.app().unwrap().id, pane);
    session.printing = Some(response(pane, "failed")?);
    assert_eq!(
      session.print_wait(Duration::from_secs(2)),
      Duration::from_millis(50)
    );
    assert!(session.print_update());
    assert!(session.app().unwrap().error.is_none());
    let origin = session.tab().panes.iter().find(|app| app.id == pane).unwrap();
    assert_eq!(
      origin.document().unwrap().error.as_deref(),
      Some("Print dialog failed")
    );
    assert!(session.printing.is_none());
    assert_eq!(
      session.print_wait(Duration::from_secs(2)),
      Duration::from_secs(2)
    );
    assert!(!session.print_update());
    // Closed panes cannot receive stale results.
    session.printing = Some(response(pane, "failed")?);
    session.activate(0)?;
    session.tab().focus = 0;
    session.close_pane()?;
    assert!(session.print_update());
    assert!(session.app().unwrap().error.is_none());
    Ok(())
  }

  #[test]
  fn shutdown_reaps_pending_job() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut session = opened(root.path())?;
    let dialog = Dialog::spawn(Command::new("/bin/sleep").arg("30"))?;
    let pid = dialog.child.id();
    session.printing = Some(Job { pane: session.app().unwrap().id, dialog });
    assert!(!session.print_update());
    session.finish()?;
    assert!(session.printing.is_none());
    assert!(
      Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()?
        .code()
        .is_some_and(|code| code != 0)
    );
    Ok(())
  }

  #[cfg(target_os = "macos")]
  #[test]
  fn print_validation() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut session = opened(root.path())?;
    assert_eq!(
      session.app().unwrap().command("print 1-2").unwrap_err().to_string(),
      "Usage: :print"
    );
    assert!(session.app().unwrap().request.is_none());
    session.app().unwrap().command("print")?;
    assert_eq!(
      session.app().unwrap().request.take(),
      Some(super::super::Request::Print)
    );
    session.printing = Some(response(session.app().unwrap().id, "cancelled")?);
    assert_eq!(
      session.print().unwrap_err().to_string(),
      "Print dialog already open"
    );
    assert!(session.print_update());
    assert!(session.app().unwrap().error.is_none());
    let path = session.app().unwrap().path.clone();
    std::fs::write(&path, "changed")?;
    assert_eq!(
      session.print().unwrap_err().to_string(),
      "PDF changed; reload before printing"
    );
    std::fs::remove_file(path)?;
    assert_eq!(
      session.print().unwrap_err().to_string(),
      "Cannot open PDF for printing"
    );
    assert!(session.printing.is_none());
    Ok(())
  }

  #[cfg(target_os = "macos")]
  #[test]
  fn pdfkit_rejects_encryption() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut session = opened(root.path())?;
    let encrypted =
      crate::encryption::encrypt(&mut session.app().unwrap().doc.native)?;
    let path = root.path().join("-quoted ' café 日本.pdf");
    std::fs::copy(encrypted.path(), &path)?;
    let before = std::fs::read(&path)?;
    let mut job = Job::start(0, &path)?;
    assert_eq!(
      complete(&mut job).unwrap_err().to_string(),
      "Encrypted PDF printing unavailable"
    );
    assert_eq!(std::fs::read(&path)?, before);
    let mut job = Job::start(0, &root.path().join("-missing ' café 日本.pdf"))?;
    assert_eq!(
      complete(&mut job).unwrap_err().to_string(),
      "Cannot open PDF for printing"
    );
    Ok(())
  }
}
