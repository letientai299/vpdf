use crate::layout::Screen;
use crate::terminal::{self, View};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::Duration;

const INTERVAL: Duration = Duration::from_millis(250);
// Client loops retain the target pane's session.
const CLIENTS: &str = "#{L:#{client_pid}:#{client_created}:\
  #{S:#{?#{==:#{session_name},#{client_session}},#{session_id},}}:\
  #{S:#{?#{==:#{session_name},#{client_session}},#{window_id},}}:\
  #{client_cell_width}:#{client_cell_height}:#{client_control_mode};}";

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct Client {
  pid: u32,
  created: u64,
  width: u16,
  height: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Snapshot {
  pub view: View,
  pub clients: Vec<Client>,
  pub screen: Option<Screen>,
}

pub(crate) fn parse(text: &str) -> Option<Snapshot> {
  let (header, clients) = text.split_once('\n')?;
  let clients = clients.trim_end_matches('\n');
  let mut fields = header.split('\t');
  let session = fields.next()?;
  let window = fields.next()?;
  let view = terminal::view(fields.next()?)?;
  if fields.next().is_some() || session.is_empty() || window.is_empty() {
    return None;
  }
  let mut attached = Vec::new();
  for row in clients.split(';').filter(|row| !row.is_empty()) {
    let fields: Vec<_> = row.split(':').collect();
    let [pid, created, owner, shown, width, height, control] = fields[..]
    else {
      return None;
    };
    if control == "1" || (owner != session && shown != window) {
      continue;
    }
    attached.push(Client {
      pid: pid.parse().ok()?,
      created: created.parse().ok()?,
      width: width.parse().ok()?,
      height: height.parse().ok()?,
    });
  }
  attached.sort_unstable();
  attached.dedup();
  Some(Snapshot { view, clients: attached, screen: None })
}

fn format() -> String {
  format!(
    "{}\n#{{session_id}}\t#{{window_id}}\t{}\n{CLIENTS}",
    crate::caps::TMUX_FORMAT,
    terminal::VIEW,
  )
}

fn query() -> Option<Snapshot> {
  let output = terminal::tmux_display(&format())?;
  let (meta, view) = output.split_once('\n')?;
  let mut snapshot = parse(view)?;
  let meta = crate::caps::tmux(meta)?;
  snapshot.screen = metrics(&meta);
  Some(snapshot)
}

fn metrics(meta: &crate::caps::Tmux) -> Option<Screen> {
  let window = crossterm::terminal::window_size().ok()?;
  terminal::screen_with(&window, Some(meta)).ok()
}

pub(crate) struct Observer {
  latest: Arc<Mutex<Option<Snapshot>>>,
  stop: mpsc::Sender<()>,
}

impl Observer {
  pub fn start() -> Option<Self> {
    if !terminal::tmux() {
      return None;
    }
    Self::watch(query)
  }

  fn watch(
    mut query: impl FnMut() -> Option<Snapshot> + Send + 'static,
  ) -> Option<Self> {
    let latest = Arc::new(Mutex::new(None));
    let result = Arc::clone(&latest);
    let (stop, stopped) = mpsc::channel();
    std::thread::Builder::new()
      .name("tmux".into())
      .spawn(move || {
        let mut previous = None;
        loop {
          if let Some(snapshot) = query()
            && previous.as_ref() != Some(&snapshot)
          {
            previous = Some(snapshot.clone());
            *result.lock().unwrap_or_else(PoisonError::into_inner) =
              Some(snapshot);
            crate::input::wake();
          }
          if stopped.recv_timeout(INTERVAL)
            != Err(mpsc::RecvTimeoutError::Timeout)
          {
            break;
          }
        }
      })
      .ok()?;
    Some(Self { latest, stop })
  }

  pub fn take(&self) -> Option<Snapshot> {
    self.latest.lock().unwrap_or_else(PoisonError::into_inner).take()
  }
}

impl Drop for Observer {
  fn drop(&mut self) {
    let _ = self.stop.send(());
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn slow_queries_preserve_state() {
    let (entered, queried) = mpsc::channel();
    let (send, receive) = mpsc::channel();
    let main = std::thread::current().id();
    let observer = Observer::watch(move || {
      entered.send(std::thread::current().id()).unwrap();
      receive.recv_timeout(Duration::from_secs(2)).ok().flatten()
    })
    .unwrap();
    let wait = || {
      let thread = queried.recv_timeout(Duration::from_secs(2)).unwrap();
      assert_ne!(thread, main);
    };
    let snapshot = parse("$0\t@0\tfocused\n10:100:$0:@0:9:26:0;\n").unwrap();
    wait();
    assert_eq!(observer.take(), None);
    send.send(Some(snapshot.clone())).unwrap();
    wait();
    assert_eq!(observer.take(), Some(snapshot.clone()));
    send.send(None).unwrap();
    wait();
    assert_eq!(observer.take(), None);
    send.send(Some(snapshot)).unwrap();
    wait();
    assert_eq!(observer.take(), None);
    drop(observer);
    send.send(None).unwrap();
  }

  #[test]
  fn observes_relevant_clients() {
    let snapshot = parse(
      "$0\t@0\tinactive\n\
       10:100:$0:@1:9:26:0;20:100:$1:@0:7:20:0;\
       30:100:$1:@1:7:20:0;40:100:$0:@0:7:20:1;",
    )
    .unwrap();
    assert_eq!(snapshot.view, View::Inactive);
    assert_eq!(snapshot.clients.len(), 2);
    assert_eq!(snapshot.clients[0].pid, 10);
    assert_eq!(snapshot.clients[1].pid, 20);
    assert_eq!(parse("$0\t@0\thidden\n").unwrap().clients, []);
    for text in ["", "$0\t@0\tunknown\n", "$0\t@0\thidden\nmalformed"] {
      assert!(parse(text).is_none(), "{text}");
    }
  }

  #[test]
  fn client_residency_tracks_identity() {
    let snapshot = |row| parse(&format!("$0\t@0\tfocused\n{row}")).unwrap();
    let first = snapshot("10:100:$0:@0:9:26:0;");
    assert_eq!(first, snapshot("10:100:$0:@0:9:26:0;"));
    assert_eq!(first.clients, snapshot("10:100:$0:@1:9:26:0;").clients);
    for row in [
      "11:100:$0:@0:9:26:0;",
      "10:101:$0:@0:9:26:0;",
      "10:100:$0:@0:7:20:0;",
      "10:100:$0:@0:9:26:0;11:100:$0:@0:9:26:0;",
      "",
    ] {
      assert_ne!(first.clients, snapshot(row).clients, "{row}");
    }
  }
}
