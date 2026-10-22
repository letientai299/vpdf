use super::{Duration, Result, Session};
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::task::Poll;

pub(super) struct Job {
  pane: u32,
  child: Child,
}

impl Job {
  fn start(pane: u32, path: &std::path::Path) -> Result<Self> {
    let child = Command::new("/usr/bin/osascript")
      .args(["-l", "JavaScript", "-e", include_str!("print.js"), "--"])
      .arg(path)
      .stdin(Stdio::null())
      .stdout(Stdio::piped())
      .stderr(Stdio::null())
      .spawn()?;
    Ok(Self { pane, child })
  }

  fn poll(&mut self) -> Result<Poll<()>> {
    let Some(status) = self.child.try_wait()? else { return Ok(Poll::Pending) };
    anyhow::ensure!(status.success(), "Print dialog failed");
    let mut output = String::new();
    if let Some(stdout) = self.child.stdout.take() {
      stdout.take(256).read_to_string(&mut output)?;
    }
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

impl Drop for Job {
  fn drop(&mut self) {
    let _ = self.child.kill();
    let _ = self.child.wait();
  }
}

impl Session {
  pub(super) fn print(&mut self) -> Result<()> {
    anyhow::ensure!(cfg!(target_os = "macos"), "Printing requires macOS");
    anyhow::ensure!(self.printing.is_none(), "Print dialog already open");
    let app = self.app();
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
      && let Some(app) = self
        .tabs
        .iter_mut()
        .flat_map(|tab| &mut tab.panes)
        .find(|app| app.id == pane)
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
    let child = Command::new("/usr/bin/printf")
      .args(["%s", output])
      .stdout(Stdio::piped())
      .spawn()?;
    Ok(Job { pane, child })
  }

  fn opened(root: &Path) -> Result<Session> {
    let path = root.join("print.pdf");
    crate::fixtures::pdf(&path, &["first", "second"], "")?;
    let mut session = Session::new(None);
    session.open(&path, true, super::super::tests::cells(120, 40))?;
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
    let child = Command::new("/usr/bin/false").spawn()?;
    assert_eq!(
      complete(&mut Job { pane: 0, child }).unwrap_err().to_string(),
      "Print dialog failed"
    );
    Ok(())
  }

  #[test]
  fn completion_tracks_origin() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut session = opened(root.path())?;
    let pane = session.app().id;
    session.app().command("vsplit")?;
    session.settle()?;
    assert_ne!(session.app().id, pane);
    let mut job = response(pane, "failed")?;
    job.child.wait()?;
    session.printing = Some(job);
    assert_eq!(
      session.print_wait(Duration::from_secs(2)),
      Duration::from_millis(50)
    );
    assert!(session.print_update());
    assert!(session.app().error.is_none());
    let origin = session.tab().panes.iter().find(|app| app.id == pane).unwrap();
    assert_eq!(origin.error.as_deref(), Some("Print dialog failed"));
    assert!(session.printing.is_none());
    assert_eq!(
      session.print_wait(Duration::from_secs(2)),
      Duration::from_secs(2)
    );
    assert!(!session.print_update());
    // Closed panes cannot receive stale results.
    let mut job = response(pane, "failed")?;
    job.child.wait()?;
    session.printing = Some(job);
    session.activate(0)?;
    session.tab().focus = 0;
    session.close_pane()?;
    assert!(session.print_update());
    assert!(session.app().error.is_none());
    Ok(())
  }

  #[test]
  fn shutdown_reaps_pending_job() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut session = opened(root.path())?;
    let child = Command::new("/bin/sleep").arg("30").spawn()?;
    let pid = child.id();
    session.printing = Some(Job { pane: session.app().id, child });
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
      session.app().command("print 1-2").unwrap_err().to_string(),
      "Usage: :print"
    );
    assert!(session.app().request.is_none());
    session.app().command("print")?;
    assert_eq!(
      session.app().request.take(),
      Some(super::super::Request::Print)
    );
    let mut job = response(session.app().id, "cancelled")?;
    job.child.wait()?;
    session.printing = Some(job);
    assert_eq!(
      session.print().unwrap_err().to_string(),
      "Print dialog already open"
    );
    assert!(session.print_update());
    assert!(session.app().error.is_none());
    let path = session.app().path.clone();
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
    let encrypted = crate::encryption::encrypt(&mut session.app().doc.native)?;
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
