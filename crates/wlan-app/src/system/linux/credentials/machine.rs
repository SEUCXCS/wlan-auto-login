use crate::infra::error::{Context, Message, Result};
use crate::system::define::credentials::CredentialMode;

use crate::system::shared::credentials::decode;
use crate::system::shared::credentials::encode;

use crate::infra::filesystem::{private_dir, write_private};
use crate::infra::paths::Paths;
use wlanlogin_lib::Credentials;

use std::{
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    sync::Mutex,
    time::SystemTime,
};

use zeroize::Zeroizing;

pub struct MachineStore {
    paths: Paths,

    mode: Option<CredentialMode>,

    cache: Mutex<Option<(FileVersion, Credentials)>>,
}

#[derive(Debug, PartialEq, Eq)]
struct FileVersion {
    generation: u64,
    device: u64,
    inode: u64,
    modified: SystemTime,
    length: u64,
}

impl FileVersion {
    fn new(metadata: &fs::Metadata, generation: u64) -> Result<Self> {
        Ok(Self {
            generation,
            device: metadata.dev(),
            inode: metadata.ino(),
            modified: metadata.modified()?,
            length: metadata.len(),
        })
    }
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
        {
            let encrypted = paths.secrets.join("login.cred").try_exists()?;
            let plain = paths.secrets.join("login.json").try_exists()?;
            crate::infra::error::ensure!(
                !(encrypted && plain),
                Message::ConflictingSystemCredentialStoresFoundRemoveObsolete
            );
            crate::infra::error::ensure!(
                !matches!(mode, Some(CredentialMode::ProtectedFile)) || !encrypted,
                Message::ExistingCredentialsAreEncryptedKeepNativeMode
            );
            crate::infra::error::ensure!(
                !matches!(mode, Some(CredentialMode::Native)) || !plain,
                Message::ExistingCredentialsUseProtectedFileBackUp
            );
        }

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

    pub async fn load(&self, generation: Option<u64>) -> Result<Option<Credentials>> {
        let open = |path: &Path| {
            fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)
        };
        let (mut file, encrypted) = match open(&self.native_file()) {
            Ok(file) => (file, true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match open(&self.plain_file()) {
                    Ok(file) => (file, false),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    Err(error) => return Err(error.into()),
                }
            }
            Err(error) => return Err(error.into()),
        };
        let metadata = file.metadata()?;
        validate_private(&metadata).map_err(|error| {
            crate::system::shared::credentials::diagnostics::specific(
                error,
                crate::infra::error::credentials::Kind::UnsafeStorage,
                crate::infra::error::credentials::Stage::Inspect,
            )
        })?;
        let version = generation
            .map(|generation| FileVersion::new(&metadata, generation))
            .transpose()?;
        if let Some(version) = &version
            && let Some((cached, credentials)) = &*self
                .cache
                .lock()
                .map_err(|_| crate::infra::error::error!(Message::CredentialCacheLockPoisoned))?
            && cached == version
        {
            return Ok(Some(credentials.clone()));
        }

        // Read the file we inspected. A unit's startup copy can predate this generation.
        let mut bytes = Zeroizing::new(Vec::new());
        file.read_to_end(&mut bytes)?;
        if encrypted {
            bytes = crypt("decrypt", &bytes).await.map_err(|error| {
                crate::system::shared::credentials::diagnostics::specific(
                    error,
                    crate::infra::error::credentials::Kind::DecryptionFailed,
                    crate::infra::error::credentials::Stage::Decrypt,
                )
            })?;
        }
        let credentials =
            decode(std::str::from_utf8(&bytes).context(Message::SystemCredentialsAreNotUTF8)?)?;
        if let Some(version) = version {
            *self
                .cache
                .lock()
                .map_err(|_| crate::infra::error::error!(Message::CredentialCacheLockPoisoned))? =
                Some((version, credentials.clone()));
        }
        Ok(Some(credentials))
    }

    pub async fn save(&self, credentials: Credentials) -> Result<()> {
        let text = encode(&credentials)?;

        {
            private_dir(&self.paths.secrets)?;

            let plain = match self.mode {
                Some(CredentialMode::ProtectedFile) => true,
                Some(CredentialMode::Native) => false,
                None => self.plain_file().try_exists()? && !self.native_file().try_exists()?,
            };

            let (path, bytes) = if plain {
                tracing::warn!("{}", Message::SystemCredentialsWillBeStoredAsPlaintext);
                (self.plain_file(), Zeroizing::new(text.as_bytes().to_vec()))
            } else {
                let bytes = crypt("encrypt", text.as_bytes()).await?;

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

fn validate_private(metadata: &fs::Metadata) -> Result<()> {
    crate::infra::error::ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        Message::CredentialPathMustBeARegularFile
    );

    {
        crate::infra::error::ensure!(
            metadata.uid() == 0 && metadata.mode() & 0o077 == 0,
            Message::SystemCredentialsMustBeOwnedByRoot
        );
    }
    Ok(())
}

pub(super) async fn crypt(operation: &str, input: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    crypt_with(std::ffi::OsStr::new("systemd-creds"), operation, input).await
}

async fn crypt_with(
    program: &std::ffi::OsStr,
    operation: &str,
    input: &[u8],
) -> Result<Zeroizing<Vec<u8>>> {
    use std::{process::Stdio, time::Duration};
    use tokio::io::AsyncWriteExt;
    let mut command = tokio::process::Command::new(program);
    command
        .args([operation, "--name=login", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .context(Message::FailedToRunSystemdCredsEncryptedCredentialsRequire)?;
    let mut stdin = child
        .stdin
        .take()
        .context(Message::FailedToOpenCredentialInputPipe)?;
    let output = tokio::time::timeout(Duration::from_secs(30), async {
        stdin.write_all(input).await?;
        drop(stdin);
        child.wait_with_output().await
    })
    .await
    .context(Message::SystemCredentialEncryptionOrDecryptionTimedOut)??;
    crate::infra::error::ensure!(
        output.status.success(),
        Message::SystemCredentialOperationFailedStorageModeWas {
            operation: (operation).to_string()
        }
    );
    Ok(Zeroizing::new(output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn delete_removes_corrupt_and_conflicting_files_without_decrypting() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path().join("SEUCXCS/wlanlogin/secrets"));
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

    #[tokio::test]
    async fn deletion_unlinks_leaf_symlinks_but_rejects_redirected_directories() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut paths = Paths::test(dir.path().join("SEUCXCS/wlanlogin/secrets"));
        fs::create_dir_all(&paths.secrets)?;
        let original = dir.path().join("outside-secret");
        fs::write(&original, "preserve")?;
        std::os::unix::fs::symlink(&original, paths.secrets.join("login.cred"))?;
        delete(&paths).await?;
        assert_eq!(fs::read_to_string(&original)?, "preserve");
        let link = dir.path().join("redirect");
        std::os::unix::fs::symlink(&paths.secrets, &link)?;
        paths.secrets = link;
        assert!(delete(&paths).await.is_err());
        Ok(())
    }

    use crate::system::shared::credentials::decode;
    use std::{fs, os::unix::fs::PermissionsExt};
    use wlanlogin_lib::Credentials;

    #[tokio::test]
    async fn secret_is_sent_to_cipher_over_stdin_and_errors_do_not_echo_it() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let helper = dir.path().join("mock-creds");
        fs::write(
            &helper,
            "#!/bin/sh\n[ \"$#\" = 4 ] || exit 1\n[ \"$2\" = --name=login ] || exit 1\ncat\n",
        )?;
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700))?;
        let secret = b"fake-test-secret";
        assert_eq!(
            &*crypt_with(helper.as_os_str(), "encrypt", secret).await?,
            secret
        );
        fs::write(&helper, "#!/bin/sh\ncat >&2\nexit 1\n")?;
        let error = crypt_with(helper.as_os_str(), "encrypt", secret)
            .await
            .err()
            .context("failure expected")?;
        assert!(!format!("{error:#}").contains("fake-test-secret"));
        Ok(())
    }

    #[tokio::test]
    async fn protected_file_is_explicit_private_and_cannot_silently_replace_encryption()
    -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path().join("private"));
        let store = MachineStore::new(paths.clone(), Some(CredentialMode::ProtectedFile))?;
        store
            .save(Credentials {
                username: "test-student".into(),
                password: "test-secret".into(),
            })
            .await?;
        let path = paths.secrets.join("login.json");
        assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(&paths.secrets)?.permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            decode(&fs::read_to_string(&path)?)?.username,
            "test-student"
        );
        assert!(MachineStore::new(paths.clone(), Some(CredentialMode::Native)).is_err());
        fs::remove_file(path)?;
        fs::write(paths.secrets.join("login.cred"), "mock encrypted data")?;
        assert!(MachineStore::new(paths, Some(CredentialMode::ProtectedFile)).is_err());
        Ok(())
    }

    #[test]
    fn machine_secret_rejects_symlinks_and_broad_file_permissions() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let secret = dir.path().join("secret");
        fs::write(&secret, "test data")?;
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o644))?;
        assert!(validate_private(&fs::symlink_metadata(&secret)?).is_err());
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o600))?;
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&secret, &link)?;
        assert!(validate_private(&fs::symlink_metadata(&link)?).is_err());
        Ok(())
    }
}
