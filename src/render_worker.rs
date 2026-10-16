use crate::document::Document;
use crate::tiles::{self, Tile, TileKey};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;

#[derive(Default)]
struct Queue {
  generation: u64,
  tiles: Vec<TileKey>,
  stop: bool,
}

pub struct RenderWorker {
  queue: Arc<(Mutex<Queue>, Condvar)>,
  pub results: mpsc::Receiver<Result<Tile, String>>,
}

impl RenderWorker {
  pub fn new(path: &Path, password: &str) -> Self {
    let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
    let work = Arc::clone(&queue);
    let (send, results) = mpsc::sync_channel(64);
    let path = path.to_owned();
    let password = password.to_owned();
    thread::spawn(move || {
      let mut doc = match Document::open(&path, &password) {
        Ok(doc) => doc,
        Err(error) => {
          let _ = send.send(Err(error.to_string()));
          return;
        }
      };
      loop {
        let (lock, wake) = &*work;
        let Ok(mut queue) = lock.lock() else { break };
        while queue.tiles.is_empty() && !queue.stop {
          queue = match wake.wait(queue) {
            Ok(q) => q,
            Err(_) => return,
          };
        }
        if queue.stop {
          break;
        }
        let generation = queue.generation;
        let tiles = std::mem::take(&mut queue.tiles);
        drop(queue);
        for key in tiles {
          let Ok(queue) = lock.lock() else { return };
          if queue.stop || queue.generation != generation {
            break;
          }
          drop(queue);
          let tile = tiles::render(&mut doc, key).map_err(|e| e.to_string());
          if send.send(tile).is_err() {
            return;
          }
        }
      }
    });
    Self { queue, results }
  }

  pub fn request(&self, tiles: &[TileKey]) {
    let (lock, wake) = &*self.queue;
    if let Ok(mut queue) = lock.lock() {
      queue.generation += 1;
      queue.tiles = tiles.to_vec();
      wake.notify_one();
    }
  }
}

impl Drop for RenderWorker {
  fn drop(&mut self) {
    let (lock, wake) = &*self.queue;
    if let Ok(mut queue) = lock.lock() {
      queue.stop = true;
      wake.notify_one();
    }
  }
}
