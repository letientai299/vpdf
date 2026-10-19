//! Background jobs and how their results reach the event loop.
use std::sync::{Arc, mpsc};

pub mod crop;
pub mod ocr;
pub mod outline;
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

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::atomic::{AtomicUsize, Ordering};

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
