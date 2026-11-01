use crate::infra::error::{Context, Message, Result};
use crate::system::define::credentials::CredentialMode;

use crate::system::shared::credentials::decode;
use crate::system::shared::credentials::encode;

use crate::infra::filesystem::windows::{machine_directory, open_machine_file, write_machine_file};
use crate::infra::paths::Paths;
use wlanlogin_lib::Credentials;

use std::{fs, io::Read, sync::Mutex, time::SystemTime};

pub struct MachineStore {
    paths: Paths,

    mode: Option<CredentialMode>,

    cache: Mutex<Option<(SystemTime, u64, Credentials)>>,
}

pub async fn delete(paths: &Paths) -> Result<()> {
    {
        // Do not construct MachineStore: its validation intentionally rejects
        // conflicting storage modes, but cleanup must handle both files.
        let _directory = match machine_directory(&paths.secrets, false) {
            Ok(guard) => guard,
            Err(error) if missing(&error) => return Ok(()),
            Err(error) => return Err(error),
        };
        let mut errors = Vec::new();
        for name in ["login.cred", "login.json"] {
            if let Err(error) = crate::infra::cleanup::remove_file(&paths.secrets.join(name)) {
                errors.push(format!("{error:#}"));
            }
        }
        crate::infra::error::ensure!(
            errors.is_empty(),
            Message::SystemCredentialDeletionFailed {
                value0: errors.join("; ")
            }
        );
        Ok(())
    }
}

impl MachineStore {
    pub fn new(paths: Paths, mode: Option<CredentialMode>) -> Result<Self> {
        crate::infra::error::ensure!(
            !matches!(mode, Some(CredentialMode::ProtectedFile)),
            Message::ProtectedFileIsOnlyAvailableOnLinux
        );

        Ok(Self {
            paths,

            mode,

            cache: Mutex::new(None),
        })
    }

    fn native_file(&self) -> std::path::PathBuf {
        self.paths.secrets.join("login.cred")
    }

    fn plain_file(&self) -> std::path::PathBuf {
        self.paths.secrets.join("login.json")
    }

    pub async fn load(&self, background: bool) -> Result<Option<Credentials>> {
        {
            // Inspect the exact open file before using either disk content or a
            // cached result, retaining the directory handles across the read.
            let opened = match open_machine_file(&self.native_file()) {
                Ok(opened) => Ok((self.native_file(), opened)),
                Err(error) if missing(&error) => {
                    open_machine_file(&self.plain_file()).map(|opened| (self.plain_file(), opened))
                }
                Err(error) => Err(error),
            };
            let (path, (mut file, _directory)) = match opened {
                Ok(opened) => opened,
                Err(error) if missing(&error) => return Ok(None),
                Err(error) if error.downcast_ref::<std::io::Error>().is_some() => {
                    return Err(crate::system::shared::credentials::diagnostics::at(
                        error,
                        crate::infra::error::credentials::Stage::Inspect,
                    ));
                }
                Err(error) => {
                    return Err(crate::system::shared::credentials::diagnostics::specific(
                        error,
                        crate::infra::error::credentials::Kind::UnsafeStorage,
                        crate::infra::error::credentials::Stage::Inspect,
                    ));
                }
            };
            let metadata = file.metadata().map_err(|error| {
                crate::system::shared::credentials::diagnostics::specific(
                    error.into(),
                    crate::infra::error::credentials::Kind::UnsafeStorage,
                    crate::infra::error::credentials::Stage::Inspect,
                )
            })?;
            let modified = metadata.modified()?;
            if background
                && let Some((old, length, credentials)) = &*self.cache.lock().map_err(|_| {
                    crate::infra::error::error!(Message::CredentialCacheLockPoisoned)
                })?
                && *old == modified
                && *length == metadata.len()
            {
                return Ok(Some(credentials.clone()));
            }

            let bytes = {
                let _ = background;
                crate::infra::error::ensure!(
                    path == self.native_file(),
                    Message::PlaintextMachineCredentialsAreNotSupportedOn
                );
                let mut encrypted = Zeroizing::new(Vec::new());
                (&mut file)
                    .take(64 * 1024 + 1)
                    .read_to_end(&mut encrypted)?;
                decrypt(&encrypted).map_err(|error| {
                    crate::system::shared::credentials::diagnostics::specific(
                        error,
                        crate::infra::error::credentials::Kind::DecryptionFailed,
                        crate::infra::error::credentials::Stage::Decrypt,
                    )
                })?
            };
            let credentials =
                decode(std::str::from_utf8(&bytes).context(Message::SystemCredentialsAreNotUTF8)?)?;
            *self
                .cache
                .lock()
                .map_err(|_| crate::infra::error::error!(Message::CredentialCacheLockPoisoned))? =
                Some((modified, metadata.len(), credentials.clone()));
            Ok(Some(credentials))
        }
    }

    pub async fn save(&self, credentials: Credentials) -> Result<()> {
        let text = encode(&credentials)?;

        {
            let plain = {
                let _ = &self.mode;
                false
            };
            let (path, bytes) = if plain {
                tracing::warn!("{}", Message::SystemCredentialsWillBeStoredAsPlaintext);
                (self.plain_file(), Zeroizing::new(text.as_bytes().to_vec()))
            } else {
                let bytes = encrypt(text.as_bytes())?;
                (self.native_file(), bytes)
            };
            let _directory = machine_directory(&self.paths.secrets, true)?;
            write_machine_file(&path, &bytes)?;
            let previous = if plain {
                self.native_file()
            } else {
                self.plain_file()
            };
            if previous.try_exists()? {
                fs::remove_file(previous)?;
            }
            *self
                .cache
                .lock()
                .map_err(|_| crate::infra::error::error!(Message::CredentialCacheLockPoisoned))? =
                None;
            Ok(())
        }
    }
}

fn missing(error: &crate::infra::error::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
}

use windows_sys::Win32::{Foundation::LocalFree, Security::Cryptography::*};
use zeroize::{Zeroize, Zeroizing};

pub fn encrypt(bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    transform(bytes, true)
}
pub fn decrypt(bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    transform(bytes, false)
}

fn transform(bytes: &[u8], encrypt: bool) -> Result<Zeroizing<Vec<u8>>> {
    crate::infra::error::ensure!(
        bytes.len() <= 64 * 1024,
        Message::CredentialPayloadExceedsTheSizeLimit
    );
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
    // SAFETY: DPAPI reads input only, allocates output with LocalAlloc, and cannot prompt.
    unsafe {
        let ok = if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        if output.cbData == 0 || output.pbData.is_null() {
            LocalFree(output.pbData.cast());
            crate::infra::error::bail!(Message::DPAPIReturnedAnEmptyCredentialPayload);
        }
        let buffer = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let bytes = Zeroizing::new(buffer.to_vec());
        buffer.zeroize();
        LocalFree(output.pbData.cast());
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn delete_removes_corrupt_and_conflicting_files_without_decrypting() -> Result<()> {
        if !crate::infra::scope::elevated()? {
            eprintln!("machine storage tests require an elevated Windows test process");
            return Ok(());
        }
        let dir = tempfile::tempdir()?;
        let mut paths = Paths::test(dir.path().join("SEUCXCS/wlanlogin/secrets"));
        paths.scope = crate::infra::scope::Scope::System;
        let _directory = machine_directory(&paths.secrets, true)?;
        for name in ["login.cred", "login.json"] {
            fs::write(paths.secrets.join(name), "invalid secret data")?;
        }
        fs::write(paths.secrets.join("unrelated"), "preserve")?;
        delete(&paths).await?;
        delete(&paths).await?;
        assert!(!paths.secrets.join("login.cred").exists());
        assert!(!paths.secrets.join("login.json").exists());
        assert_eq!(
            fs::read_to_string(paths.secrets.join("unrelated"))?,
            "preserve"
        );
        Ok(())
    }
}
