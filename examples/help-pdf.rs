use anyhow::{Context, Result};
use std::sync::mpsc;
use std::time::Duration;
use vpdf::config::Settings;
use vpdf::manual::Runtime;
use vpdf_pdf::jobs::Wake;
use vpdf_ui::topics::{Invocation, TopicId};

fn main() -> Result<()> {
  let destination = std::env::args_os().nth(1).context("Missing output PDF")?;
  let reference = Settings::default().help_reference(Invocation::Home);
  let (send, receive) = mpsc::channel();
  let wake = Wake::new(move || {
    let _ = send.send(());
  });
  let mut runtime = Runtime::default();
  runtime.request(&reference, TopicId::parse("introduction")?, &wake);
  receive.recv_timeout(Duration::from_secs(60))?;
  let ready = runtime.poll().context("Missing help document")??;
  std::fs::copy(ready.document.path(), destination)?;
  Ok(())
}
