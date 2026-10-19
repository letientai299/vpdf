use vpdf_reader::reader::Reader;
use vpdf_ui::reading::Pane;

pub trait SeedReader {
  fn seed_reader(&mut self, seed: impl FnOnce(&mut Reader));
}

impl SeedReader for Pane {
  fn seed_reader(&mut self, seed: impl FnOnce(&mut Reader)) {
    let mut reader = (**self).clone();
    seed(&mut reader);
    *self = Self::new(reader, self.tracking().clone());
  }
}
