fn main() -> vpdf_ui::Result<()> {
  println!("{}", vpdf_ui::topics::build::facts()?);
  Ok(())
}
