use super::*;
use std::sync::mpsc;
use std::time::Duration;
use vpdf_pdf::jobs::OneShot;
use vpdf_ui::topics::Invocation;

#[test]
fn discards_obsolete_jobs() -> Result<()> {
  for queued in [false, true] {
    let reference =
      crate::config::Settings::default().help_reference(Invocation::Home);
    let (send, receive) = mpsc::channel();
    let wake = Wake::new(move || {
      let _ = send.send(());
    });
    let (release, blocked) = mpsc::channel();
    let (finished, done) = mpsc::channel();
    let job = OneShot::spawn(&wake, move |stop| {
      if queued {
        Some(Err(anyhow::anyhow!("Obsolete failure")))
      } else {
        blocked.recv().unwrap();
        assert!(stop.load(std::sync::atomic::Ordering::Relaxed));
        finished.send(()).unwrap();
        None
      }
    });
    if queued {
      receive.recv_timeout(Duration::from_secs(10))?;
    }
    let mut runtime = Runtime {
      state: State::Building { key: Key { manual: "old-manual".into() }, job },
      selected: Some(Location::topic(TopicId::parse("half_down")?)),
      delivery: Some(Delivery::Open),
    };
    runtime.request(&reference, TopicId::parse("config")?, &wake);
    if !queued {
      release.send(())?;
      done.recv_timeout(Duration::from_secs(10))?;
    }
    let ready = loop {
      if let Some(result) = runtime.poll() {
        break result?;
      }
      receive.recv_timeout(Duration::from_secs(10))?;
    };
    assert_eq!(ready.location.id().as_str(), "config");
    assert!(runtime.poll().is_none());
  }
  Ok(())
}

#[test]
fn failed_generation_retries() -> Result<()> {
  let reference =
    crate::config::Settings::default().help_reference(Invocation::Home);
  let (send, receive) = mpsc::channel();
  let wake = Wake::new(move || {
    let _ = send.send(());
  });
  let job: Job =
    OneShot::spawn(&wake, |_| Some(Err(anyhow::anyhow!("Generation failed"))));
  receive.recv_timeout(Duration::from_secs(10))?;
  let mut runtime = Runtime {
    state: State::Building { key: Key::new(), job },
    selected: Some(Location::topic(TopicId::parse("config")?)),
    delivery: Some(Delivery::Open),
  };
  let error = runtime.poll().unwrap().err().unwrap();
  assert_eq!(error.to_string(), "Generation failed");
  assert!(matches!(runtime.state, State::Empty));
  runtime.refresh(&reference, &wake);
  assert!(matches!(runtime.state, State::Building { .. }));
  Ok(())
}
