//! Native callbacks enqueue worker inputs.
use eframe::egui;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use vpdf::session::native::Input;
use vpdf::session::native::commands::{Action, Effect, Snapshot};

pub struct Native {
  integration: vpdf_macos::Integration,
  snapshot: Rc<RefCell<Arc<Snapshot>>>,
  inputs: Rc<RefCell<Vec<Input>>>,
  context: egui::Context,
}

impl Native {
  pub fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
    let snapshot = Rc::new(RefCell::new(Arc::new(Snapshot::default())));
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let current = Rc::clone(&snapshot);
    let pending = Rc::clone(&inputs);
    let context = cc.egui_ctx.clone();
    let wake = context.clone();
    let entries: Vec<_> = MENUS
      .iter()
      .map(|entry| vpdf_macos::Item {
        menu: entry.item.menu,
        title: entry.item.title,
        key: entry.item.key,
        shift: entry.item.shift,
        control: entry.item.control,
      })
      .collect();
    let integration =
      vpdf_macos::Integration::install(cc, &entries, move |request| {
        let input = match request {
          vpdf_macos::Request::Menu(index) => MENUS
            .get(index)
            .and_then(|entry| current.borrow().command(entry.action))
            .map(Input::Action),
          vpdf_macos::Request::Open(path) => Some(Input::Open(path)),
        };
        if let Some(input) = input {
          pending.borrow_mut().push(input);
          wake.request_repaint();
        }
      })?;
    Some(Self { integration, snapshot, inputs, context })
  }

  pub fn update(&mut self, snapshot: &Arc<Snapshot>, effects: Vec<Effect>) {
    if !Arc::ptr_eq(&self.snapshot.borrow(), snapshot) {
      *self.snapshot.borrow_mut() = Arc::clone(snapshot);
      for (index, entry) in MENUS.iter().enumerate() {
        self
          .integration
          .enable(index, snapshot.command(entry.action).is_some());
      }
      self.integration.enable_recent(snapshot.command(Action::Open).is_some());
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

  pub fn restorer(&self) -> impl Fn() + 'static {
    self.integration.restorer()
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
    item: vpdf_macos::Item { menu, title, key, shift: false, control: false },
    action,
  }
}

pub const MENUS: &[Entry] = &[
  entry("File", "Open…", "o", Action::Open),
  entry("File", "New Tab", "t", Action::NewTab),
  entry("File", "Close Tab", "w", Action::CloseTab),
  entry("File", "Reveal in Finder", "", Action::Reveal),
  entry("File", "Print…", "p", Action::Print),
  entry("Edit", "Undo", "z", Action::Undo),
  entry("Edit", "Cut", "x", Action::Cut),
  entry("Edit", "Copy", "c", Action::Copy),
  entry("Edit", "Paste", "v", Action::Paste),
  entry("Edit", "Select All", "a", Action::SelectAll),
  entry("Edit", "Find…", "f", Action::Find),
  entry("Edit", "Find Next", "g", Action::NextResult),
  Entry {
    item: vpdf_macos::Item {
      menu: "Edit",
      title: "Find Previous",
      key: "g",
      shift: true,
      control: false,
    },
    action: Action::PreviousResult,
  },
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
    },
    action: Action::PreviousTab,
  },
  entry("Window", "Close Pane", "", Action::ClosePane),
  entry("Help", "Vpdf Help", "", Action::Help),
];

pub fn shortcuts() -> Vec<egui::KeyboardShortcut> {
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
    .collect()
}
