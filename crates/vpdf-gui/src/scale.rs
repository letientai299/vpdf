use eframe::egui;
use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone)]
pub struct Scale {
  value: Rc<Cell<f32>>,
  context: egui::Context,
}

impl Scale {
  pub fn new(context: &egui::Context) -> Self {
    context.options_mut(|options| options.zoom_with_keyboard = false);
    Self {
      value: Rc::new(Cell::new(vpdf_macos::ui_scale().unwrap_or(1.0))),
      context: context.clone(),
    }
  }

  pub fn value(&self) -> f32 {
    self.value.get()
  }

  pub fn set(&self, value: f32) {
    self.value.set(value);
    vpdf_macos::set_ui_scale(value);
    self.context.request_repaint();
  }

  pub fn step(&self, direction: f32) {
    self.set(crate::input::scale_step(self.value(), direction));
  }

  pub fn apply(&self) {
    self.context.set_zoom_factor(self.value());
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn pending_changes_survive_frame_boundaries() {
    let context = egui::Context::default();
    let scale =
      Scale { value: Rc::new(Cell::new(1.0)), context: context.clone() };
    let other = scale.clone();
    scale.value.set(crate::input::scale_step(scale.value(), -1.0));
    other.value.set(crate::input::scale_step(other.value(), -1.0));
    let mut output =
      context.run_ui(egui::RawInput::default(), |_| scale.apply());
    output.textures_delta.clear();
    assert_eq!(other.value(), 0.8);
    other.value.set(crate::input::scale_step(other.value(), 1.0));
    let mut output =
      context.run_ui(egui::RawInput::default(), |_| other.apply());
    output.textures_delta.clear();
    let mut output =
      context.run_ui(egui::RawInput::default(), |_| scale.apply());
    output.textures_delta.clear();
    assert_eq!(context.zoom_factor(), 0.9);
  }
}
