//! Read the one-use form encrypted by the immediate MSI action.
use crate::{Form, Scope, win};
use anyhow::{Result, ensure};
use std::{fs, path::Path};

pub fn receive(file: &str, transaction: &str, scope: Scope) -> Result<Form> {
    let path = Path::new(file);
    validate_path(path, transaction)?;
    win::plain_path(path)?;
    ensure!(
        fs::metadata(path)?.len() <= 64 * 1024,
        "Encrypted credential form exceeds size limit"
    );
    let data = win::protect(&fs::read(path)?, scope == Scope::Machine, true)?;
    let mut input = data.as_slice();
    let mut field = || -> Result<String> {
        ensure!(input.len() >= 4, "Truncated credential form");
        let length = u32::from_le_bytes(input[..4].try_into()?) as usize;
        input = &input[4..];
        ensure!(length <= input.len(), "Truncated credential field");
        let value = std::str::from_utf8(&input[..length])?.to_string();
        input = &input[length..];
        Ok(value)
    };
    let form = Form {
        username: field()?,
        password: field()?,
        line: field()?,
    };
    ensure!(input.is_empty(), "Unexpected credential form data");
    form.validate()?;
    cleanup(file, transaction)?;
    Ok(form)
}

fn validate_path(path: &Path, transaction: &str) -> Result<()> {
    crate::validate_id(transaction)?;
    ensure!(
        path.file_name()
            .is_some_and(|name| name == "credentials.bin")
            && path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == format!("wlanlogin-form-{transaction}").as_str()),
        "Unexpected credential form location"
    );
    Ok(())
}
pub fn cleanup(file: &str, transaction: &str) -> Result<()> {
    if file.is_empty() {
        return Ok(());
    }
    let path = Path::new(file);
    validate_path(path, transaction)?;
    crate::state::remove_file(path)?;
    if let Some(directory) = path.parent() {
        match fs::remove_dir(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_one_use_form_round_trips_without_native_credentials() -> Result<()> {
        let root = tempfile::tempdir()?;
        let transaction = "397f9b01f368468bad7c23822e646f3300";
        let directory = root.path().join(format!("wlanlogin-form-{transaction}"));
        win::private_directory(&directory, false)?;
        let path = directory.join("credentials.bin");
        let mut bytes = zeroize::Zeroizing::new(Vec::new());
        for field in ["test-user", "test-password", "free2"] {
            bytes.extend_from_slice(&(field.len() as u32).to_le_bytes());
            bytes.extend_from_slice(field.as_bytes());
        }
        fs::write(&path, win::protect(&bytes, false, false)?)?;
        let form = receive(&path.to_string_lossy(), transaction, Scope::User)?;
        assert_eq!(form.username, "test-user");
        assert_eq!(form.password, "test-password");
        assert_eq!(form.line, "free2");
        assert!(!directory.exists());
        cleanup(&path.to_string_lossy(), transaction)?;
        assert!(
            cleanup(
                &root.path().join("unrelated").to_string_lossy(),
                transaction
            )
            .is_err()
        );
        Ok(())
    }
}
