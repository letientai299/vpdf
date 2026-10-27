//! Native callbacks enqueue worker inputs.
use eframe::egui;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use vpdf::session::native::Input;
use vpdf::session::native::commands::{Action, Command, Effect, Snapshot};

pub struct Native {
  integration: vpdf_macos::Integration,
  command: Rc<Cell<Option<Command>>>,
  inputs: Rc<RefCell<Vec<Input>>>,
  context: egui::Context,
}

impl Native {
  pub fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
    let command = Rc::new(Cell::new(None));
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let current = Rc::clone(&command);
    let pending = Rc::clone(&inputs);
    let context = cc.egui_ctx.clone();
    let wake = context.clone();
    let integration = vpdf_macos::Integration::install(cc, move || {
      if let Some(command) = current.get() {
        pending.borrow_mut().push(Input::Action(command));
        wake.request_repaint();
      }
    })?;
    Some(Self { integration, command, inputs, context })
  }

  pub fn update(&mut self, snapshot: &Snapshot, effects: Vec<Effect>) {
    let command = snapshot.command(Action::Open);
    self.command.set(command);
    self.integration.enable(command.is_some());
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
      }
    }
  }

  pub fn drain(&self) -> Vec<Input> {
    std::mem::take(&mut *self.inputs.borrow_mut())
  }

  #[cfg(feature = "bench-events")]
  pub fn probe(&mut self, command: &str) {
    match command {
      "menu" => self.integration.invoke_menu(),
      "key" => {
        println!("native-key: {}", self.integration.invoke_key());
      }
      "cancel" => self.integration.cancel(),
      "inspect" => println!("native-state: {:?}", self.integration.inspect()),
      _ => eprintln!("Unknown native probe"),
    }
    println!("native-pending: {}", self.inputs.borrow().len());
  }

  pub fn cancel(&mut self) {
    self.integration.cancel();
  }
}
