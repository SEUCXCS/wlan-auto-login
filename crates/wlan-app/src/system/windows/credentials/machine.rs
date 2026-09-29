use crate::infra::error::{Context, Message, Result};
use crate::system::define::credentials::CredentialMode;

use crate::system::shared::credentials::decode;
use crate::system::shared::credentials::encode;

use crate::infra::filesystem::{private_dir, write_private};
use crate::infra::paths::Paths;
use cxxy_wlanlogin_lib::Credentials;

use std::{fs, path::Path, sync::Mutex, time::SystemTime};

pub struct MachineStore {
    paths: Paths,

    mode: Option<CredentialMode>,

    cache: Mutex<Option<(SystemTime, u64, Credentials)>>,
}

pub async fn delete(paths: &Paths) -> Result<()> {
    {
        // Do not construct MachineStore: its validation intentionally rejects
        // conflicting storage modes, but cleanup must handle both files.
        crate::infra::cleanup::validate_directory(&paths.secrets)?;
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
            let path = if self.native_file().try_exists()? {
                self.native_file()
            } else {
                self.plain_file()
            };
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            validate_private(&path)?;
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
                decrypt(&fs::read(&path)?)?
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
            private_dir(&self.paths.secrets)?;

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
            write_private(&path, &bytes)?;
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

fn validate_private(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    crate::infra::error::ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        Message::CredentialPathMustBeARegularFile
    );

    Ok(())
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
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path().join("cxxy-wlanlogin/secrets"));
        fs::create_dir_all(&paths.secrets)?;
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
