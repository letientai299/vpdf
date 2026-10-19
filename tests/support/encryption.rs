use anyhow::Result;
use mupdf::pdf::{Encryption, PdfDocument, PdfWriteOptions};

pub fn encrypt(source: &mut PdfDocument) -> Result<tempfile::NamedTempFile> {
  let mut options = PdfWriteOptions::default();
  options
    .set_encryption(Encryption::Aes256)
    .set_owner_password("test-owner")
    .set_user_password("test-reader");
  let mut file = tempfile::NamedTempFile::new()?;
  source.write_to_with_options(&mut file, options)?;
  Ok(file)
}
