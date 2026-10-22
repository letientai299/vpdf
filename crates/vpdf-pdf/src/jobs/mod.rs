//! Background jobs and how their results reach the event loop.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

pub mod capture;
pub mod crop;
pub mod figures;
pub mod ocr;
pub mod outline;
pub mod recent;
pub mod render;
pub mod search;

/// Wakes the event loop after a job sends a result.
#[derive(Clone)]
pub struct Wake(Arc<dyn Fn() + Send + Sync>);

impl Wake {
  pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
    Self(Arc::new(wake))
  }

  /// A wake for jobs no loop waits on.
  pub fn none() -> Self {
    Self::new(|| {})
  }

  pub fn wake(&self) {
    (self.0)();
  }

  /// `send`, waking the loop after each value.
  pub fn sender<S>(&self, send: S) -> WakingSender<S> {
    WakingSender { send, wake: self.clone() }
  }
}

/// A channel's sending half.
pub trait Channel<T> {
  fn put(&self, value: T) -> Result<(), mpsc::SendError<T>>;
}

impl<T> Channel<T> for mpsc::Sender<T> {
  fn put(&self, value: T) -> Result<(), mpsc::SendError<T>> {
    self.send(value)
  }
}

impl<T> Channel<T> for mpsc::SyncSender<T> {
  fn put(&self, value: T) -> Result<(), mpsc::SendError<T>> {
    self.send(value)
  }
}

/// A result channel whose sends wake the event loop; the value is queued
/// before the loop wakes to read it.
#[derive(Clone)]
pub struct WakingSender<S> {
  send: S,
  wake: Wake,
}

impl<S> WakingSender<S> {
  pub fn send<T>(&self, value: T) -> Result<(), mpsc::SendError<T>>
  where
    S: Channel<T>,
  {
    let sent = self.send.put(value);
    self.wake.wake();
    sent
  }
}

/// A cancellable worker with optional delivery.
pub struct OneShot<T> {
  pub result: mpsc::Receiver<T>,
  stop: Arc<AtomicBool>,
}

impl<T: Send + 'static> OneShot<T> {
  pub fn spawn(
    wake: &Wake,
    work: impl FnOnce(&AtomicBool) -> Option<T> + Send + 'static,
  ) -> Self {
    let (send, receive) = mpsc::channel();
    let send = wake.sender(send);
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::clone(&stop);
    std::thread::spawn(move || {
      if let Some(result) = work(&stopped) {
        let _ = send.send(result);
      }
    });
    Self { result: receive, stop }
  }
}

impl<T> OneShot<T> {
  pub fn cancel(&self) {
    self.stop.store(true, Ordering::Relaxed);
  }

  pub fn canceled(&self) -> bool {
    self.stop.load(Ordering::Relaxed)
  }
}

impl<T> Drop for OneShot<T> {
  fn drop(&mut self) {
    self.cancel();
  }
}

/// Shared CPU helper limits.
#[derive(Clone, Copy)]
pub struct Config {
  pub helpers: std::num::NonZeroUsize,
  pub prefetch_pages: usize,
}

impl Default for Config {
  fn default() -> Self {
    static CONFIG: std::sync::OnceLock<Config> = std::sync::OnceLock::new();
    *CONFIG.get_or_init(|| Self {
      prefetch_pages: 10,
      helpers: std::num::NonZeroUsize::new(
        std::thread::available_parallelism()
          .map_or(2, |count| count.get() / 2)
          .clamp(1, 4),
      )
      .unwrap(),
    })
  }
}

/// Owner-scoped work surviving a service replacement.
#[derive(Clone)]
pub struct Activity(Arc<ActivityState>);

struct ActivityState {
  active: AtomicBool,
  wake: std::sync::Mutex<Option<Wake>>,
}

impl Activity {
  pub fn new(active: bool) -> Self {
    Self(Arc::new(ActivityState {
      active: AtomicBool::new(active),
      wake: std::sync::Mutex::new(None),
    }))
  }
  pub fn active(&self) -> bool {
    self.0.active.load(Ordering::Acquire)
  }
  pub fn observe(&self, wake: &Wake) {
    if let Ok(mut slot) = self.0.wake.lock() {
      *slot = Some(wake.clone());
    }
  }
  pub fn same(&self, other: &Self) -> bool {
    Arc::ptr_eq(&self.0, &other.0)
  }
  pub(super) fn start(&self) {
    self.0.active.store(true, Ordering::Release);
  }
  pub(super) fn guard(&self) -> ActivityGuard {
    ActivityGuard(self.clone())
  }
}

pub(super) struct ActivityGuard(Activity);

impl Drop for ActivityGuard {
  fn drop(&mut self) {
    self.0.0.active.store(false, Ordering::Release);
    if let Ok(wake) = self.0.0.wake.lock()
      && let Some(wake) = &*wake
    {
      wake.wake();
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::atomic::AtomicUsize;
  use std::time::Duration;

  #[test]
  fn one_shot_lifecycle() {
    for deliver in [false, true] {
      let (wake, woken) = crate::fixtures::wake();
      let job = OneShot::spawn(&wake, move |_| deliver.then_some(42));
      if deliver {
        woken.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(job.result.try_recv().unwrap(), 42);
      } else {
        assert!(matches!(
          job.result.recv_timeout(Duration::from_secs(5)),
          Err(mpsc::RecvTimeoutError::Disconnected)
        ));
        assert!(woken.try_recv().is_err());
      }
    }
  }

  #[test]
  fn cancel_drop_detaches() {
    for cancel_first in [false, true] {
      let (release, blocked) = mpsc::channel();
      let (wake, woken) = crate::fixtures::wake();
      let job = OneShot::spawn(&wake, move |stop| {
        blocked.recv().unwrap();
        assert!(stop.load(Ordering::Relaxed));
        Some(())
      });
      if cancel_first {
        job.cancel();
        assert!(job.canceled());
      }
      let stop = Arc::clone(&job.stop);
      drop(job);
      assert!(stop.load(Ordering::Relaxed));
      release.send(()).unwrap();
      woken.recv_timeout(Duration::from_secs(5)).unwrap();
    }
  }

  #[test]
  fn sends_queue_before_waking() {
    let (send, receive) = mpsc::channel();
    let seen = Arc::new(AtomicUsize::new(0));
    let woken = Arc::new(AtomicUsize::new(0));
    let (count, ready) = (Arc::clone(&woken), Arc::clone(&seen));
    let receive = std::sync::Mutex::new(receive);
    let wake = Wake::new(move || {
      let queued = receive.lock().unwrap().try_iter().count();
      ready.fetch_add(queued, Ordering::Relaxed);
      count.fetch_add(1, Ordering::Relaxed);
    });
    let send = wake.sender(send);
    send.send(1).unwrap();
    send.send(2).unwrap();
    assert_eq!(woken.load(Ordering::Relaxed), 2);
    assert_eq!(seen.load(Ordering::Relaxed), 2, "each value preceded its wake");
  }

  #[test]
  fn closed_channels_still_wake() {
    let (send, receive) = mpsc::sync_channel(1);
    drop(receive);
    let woken = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&woken);
    let wake = Wake::new(move || {
      count.fetch_add(1, Ordering::Relaxed);
    });
    assert!(wake.sender(send).send(()).is_err());
    assert_eq!(woken.load(Ordering::Relaxed), 1);
  }
}
