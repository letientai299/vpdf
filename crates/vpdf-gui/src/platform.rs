//! Native callbacks enqueue worker inputs.
use eframe::egui;
use std::cell::{OnceCell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use vpdf::session::native::commands::{Action, Effect, Snapshot};
use vpdf::session::native::{Input, Sender};

pub struct Native {
  integration: vpdf_macos::Integration,
  snapshot: Rc<RefCell<Arc<Snapshot>>>,
  inputs: Rc<RefCell<Vec<Input>>>,
  context: egui::Context,
}

impl Native {
  /// Finder events in `documents` reach `sender` without a frame, so
  /// minimized and hidden windows still open and quit.
  pub fn new(
    cc: &eframe::CreationContext<'_>,
    documents: &fmv_macos_events::Inbox,
    sender: Sender,
  ) -> Option<Self> {
    let snapshot = Rc::new(RefCell::new(Arc::new(Snapshot::default())));
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let current = Rc::clone(&snapshot);
    let pending = Rc::clone(&inputs);
    let context = cc.egui_ctx.clone();
    let wake = context.clone();
    let restore: Rc<Restorer> = Rc::default();
    let restorer = Rc::clone(&restore);
    let inbox = documents.clone();
    let entries: Vec<_> = MENUS
      .iter()
      .map(|entry| vpdf_macos::Item {
        menu: entry.item.menu,
        title: entry.item.title,
        key: entry.item.key,
        shift: entry.item.shift,
        control: entry.item.control,
        responder: entry.item.responder,
      })
      .collect();
    let integration = vpdf_macos::Integration::install(
      cc,
      &entries,
      &tools(),
      move |request| {
        let input = match request {
          vpdf_macos::Request::Menu(index) => MENUS
            .get(index)
            .and_then(|entry| current.borrow().command(entry.action))
            .map(Input::Action),
          vpdf_macos::Request::Page(page) => current
            .borrow()
            .command(Action::GoToPage)
            .map(|command| Input::Page { command, page }),
          vpdf_macos::Request::Open(path) => Some(Input::Open(path)),
          // Outside Eframe's handler, so AppKit may end the loop.
          vpdf_macos::Request::Deferred => {
            let open = |path| reopen(&restorer, &sender, path);
            route(&inbox, open, vpdf_macos::terminate);
            None
          }
        };
        if let Some(input) = input {
          pending.borrow_mut().push(input);
          wake.request_repaint();
        }
      },
    )?;
    let _ = restore.set(Box::new(integration.restorer()));
    documents.set_wake(integration.deferrer());
    Some(Self { integration, snapshot, inputs, context })
  }

  pub fn update(&mut self, snapshot: &Arc<Snapshot>, effects: Vec<Effect>) {
    if !Arc::ptr_eq(&self.snapshot.borrow(), snapshot) {
      *self.snapshot.borrow_mut() = Arc::clone(snapshot);
      let states: Vec<_> = MENUS
        .iter()
        .map(|entry| snapshot.command(entry.action).is_some())
        .collect();
      self.integration.enable(&states);
      self.integration.enable_recent(snapshot.command(Action::Open).is_some());
      self.integration.retitle(
        index(Action::Sidebar),
        if snapshot.panel.is_some() { "Hide Sidebar" } else { "Show Sidebar" },
      );
      self.integration.page(
        snapshot.location.map(|location| (location.page.0 + 1, snapshot.pages)),
        snapshot.command(Action::GoToPage).is_some(),
      );
      // Help has a builtin, relative identity.
      self
        .integration
        .represent(snapshot.path.as_deref().filter(|path| path.is_absolute()));
    }
    for effect in effects {
      match effect {
        Effect::Open { request } => {
          let inputs = Rc::clone(&self.inputs);
          let context = self.context.clone();
          self.integration.pick(move |paths| {
            inputs.borrow_mut().push(Input::Picked { request, paths });
            context.request_repaint();
          });
        }
        Effect::Recent(path) => self.integration.note_recent(&path),
        Effect::Reveal(path) => self.integration.reveal(&path),
        Effect::EditConfig(path) => self.integration.edit(&path),
      }
    }
  }

  pub fn drain(&self) -> Vec<Input> {
    std::mem::take(&mut *self.inputs.borrow_mut())
      .into_iter()
      .map(|input| {
        if let Input::Action(command) = input
          && command.action == Action::Paste
        {
          return Input::PasteTarget {
            command,
            text: self.integration.paste().unwrap_or_default(),
          };
        }
        input
      })
      .collect()
  }

  #[cfg(feature = "bench-events")]
  pub fn probe(&mut self, command: &str) {
    match command {
      "menu" => self.integration.invoke_menu("Open…"),
      "key" => {
        println!("native-key: {}", self.integration.invoke_key("Open…"));
      }
      "cancel" => self.integration.cancel(),
      "inspect" => println!("native-state: {:?}", self.integration.inspect()),
      "recent" => self.integration.invoke_recent(),
      "represented" => {
        println!("native-represented: {:?}", self.integration.represented());
      }
      "minimize" => self.integration.minimize(),
      "minimized" => {
        println!("native-minimized: {}", self.integration.minimized());
      }
      "toolbar" => {
        println!("native-toolbar: {}", self.integration.toolbar_state());
      }
      "app menu" => {
        println!("native-app: {:?}", self.integration.application_menu());
      }
      "edit page" => {
        println!("native-editing: {}", self.integration.edit_page());
      }
      command if command.starts_with("tool ") => {
        self.integration.invoke_tool(&command[5..]);
      }
      command if command.starts_with("page ") => {
        self.integration.submit_page(&command[5..], false);
      }
      command if command.starts_with("unpage ") => {
        self.integration.submit_page(&command[7..], true);
      }
      command if command.starts_with("menu ") => {
        self.integration.invoke_menu(&command[5..]);
      }
      command if command.starts_with("key ") => {
        println!("native-key: {}", self.integration.invoke_key(&command[4..]));
      }
      _ => eprintln!("Unknown native probe"),
    }
    println!("native-pending: {}", self.inputs.borrow().len());
  }

  pub fn cancel(&mut self) {
    self.integration.cancel();
  }
}

/// Set once the adapter can find its window.
type Restorer = OnceCell<Box<dyn Fn()>>;

/// Shows a minimized window for an active app, then opens `path`.
fn reopen(restore: &Restorer, sender: &Sender, path: std::path::PathBuf) {
  if let Some(restore) = restore.get() {
    restore();
  }
  sender.send(Input::Open(path));
}

/// Hands each queued Finder open and quit to its handler.
pub fn route(
  documents: &fmv_macos_events::Inbox,
  mut open: impl FnMut(std::path::PathBuf),
  mut quit: impl FnMut(),
) {
  for event in documents.drain() {
    match event {
      fmv_macos_events::Event::Open(paths) => {
        paths.into_iter().for_each(&mut open);
      }
      fmv_macos_events::Event::Quit => quit(),
    }
  }
}

// Menu order also identifies callback commands.
pub struct Entry {
  pub item: vpdf_macos::Item,
  pub action: Action,
}

const fn entry(
  menu: &'static str,
  title: &'static str,
  key: &'static str,
  action: Action,
) -> Entry {
  Entry {
    item: vpdf_macos::Item {
      menu,
      title,
      key,
      shift: false,
      control: false,
      responder: None,
    },
    action,
  }
}

/// An Edit entry that page-field editing sends to `responder`.
const fn edit(
  title: &'static str,
  key: &'static str,
  action: Action,
  responder: &'static std::ffi::CStr,
) -> Entry {
  let mut entry = entry("Edit", title, key, action);
  entry.item.responder = Some(responder);
  entry
}

fn index(action: Action) -> usize {
  MENUS
    .iter()
    .position(|entry| entry.action == action)
    .expect("Menu entry for action")
}

fn tools() -> Vec<vpdf_macos::Tool> {
  use vpdf_macos::Tool;
  let button = |action, label, symbol| Tool::Button {
    entry: index(action),
    label,
    symbol,
    keep: false,
  };
  vec![
    button(Action::Sidebar, "Sidebar", "sidebar.left"),
    Tool::Page,
    Tool::Space,
    button(Action::ZoomOut, "Zoom Out", "minus.magnifyingglass"),
    button(Action::ZoomIn, "Zoom In", "plus.magnifyingglass"),
    Tool::Menu {
      label: "Fit",
      symbol: "arrow.up.left.and.arrow.down.right",
      entries: vec![index(Action::FitWidth), index(Action::FitPage)],
    },
    Tool::Button {
      entry: index(Action::Find),
      label: "Find",
      symbol: "magnifyingglass",
      keep: true,
    },
  ]
}

pub const MENUS: &[Entry] = &[
  entry("File", "Open…", "o", Action::Open),
  entry("File", "New Tab", "t", Action::NewTab),
  entry("File", "Close Tab", "w", Action::CloseTab),
  entry("File", "Reveal in Finder", "", Action::Reveal),
  entry("File", "Print…", "p", Action::Print),
  edit("Undo", "z", Action::Undo, c"undo:"),
  edit("Cut", "x", Action::Cut, c"cut:"),
  edit("Copy", "c", Action::Copy, c"copy:"),
  edit("Paste", "v", Action::Paste, c"paste:"),
  edit("Select All", "a", Action::SelectAll, c"selectAll:"),
  entry("Edit", "Find…", "f", Action::Find),
  entry("Edit", "Find Next", "g", Action::NextResult),
  Entry {
    item: vpdf_macos::Item {
      menu: "Edit",
      title: "Find Previous",
      key: "g",
      shift: true,
      control: false,
      responder: None,
    },
    action: Action::PreviousResult,
  },
  entry("View", "Show Sidebar", "", Action::Sidebar),
  entry("View", "Fit Width", "", Action::FitWidth),
  entry("View", "Fit Page", "", Action::FitPage),
  entry("View", "Zoom In", "+", Action::ZoomIn),
  entry("View", "Zoom Out", "-", Action::ZoomOut),
  entry("View", "Rotate Clockwise", "", Action::Rotate),
  entry("View", "Back", "[", Action::Back),
  entry("View", "Forward", "]", Action::Forward),
  entry("View", "Outline", "", Action::Outline),
  entry("View", "Overview", "", Action::Overview),
  Entry {
    item: vpdf_macos::Item {
      menu: "Window",
      title: "Next Tab",
      key: "\t",
      shift: false,
      control: true,
      responder: None,
    },
    action: Action::NextTab,
  },
  Entry {
    item: vpdf_macos::Item {
      menu: "Window",
      title: "Previous Tab",
      key: "\t",
      shift: true,
      control: true,
      responder: None,
    },
    action: Action::PreviousTab,
  },
  entry("Window", "Close Pane", "", Action::ClosePane),
  entry("Help", "Vpdf Help", "", Action::Help),
  entry("", "Settings…", ",", Action::Settings),
];

pub fn shortcuts() -> Vec<egui::KeyboardShortcut> {
  // The adapter's Hide Toolbar item.
  let toolbar = egui::KeyboardShortcut::new(
    egui::Modifiers::MAC_CMD | egui::Modifiers::ALT,
    egui::Key::T,
  );
  MENUS
    .iter()
    .filter_map(|entry| {
      let key = match entry.item.key {
        "\t" => Some(egui::Key::Tab),
        "" => None,
        key => egui::Key::from_name(key),
      }?;
      let mut modifiers = if entry.item.control {
        egui::Modifiers::CTRL
      } else {
        egui::Modifiers::MAC_CMD
      };
      modifiers.shift = entry.item.shift;
      Some(egui::KeyboardShortcut::new(modifiers, key))
    })
    .chain([toolbar])
    .collect()
}
