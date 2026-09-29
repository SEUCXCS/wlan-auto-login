#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::crypt;
#[cfg(target_os = "linux")]
pub use linux::linux_credential_directive;

#[cfg(not(target_os = "macos"))]
use super::decode;
use super::encode;
#[cfg(not(target_os = "macos"))]
use crate::infra::filesystem::{private_dir, write_private};
use crate::infra::paths::Paths;
#[cfg(not(target_os = "macos"))]
use anyhow::Context;
use anyhow::Result;
use cxxy_wlanlogin_lib::Credentials;
#[cfg(not(target_os = "macos"))]
use std::{fs, path::Path, sync::Mutex, time::SystemTime};
#[cfg(not(target_os = "macos"))]
use zeroize::Zeroizing;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::authorize_binary;
#[cfg(windows)]
mod windows;

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum CredentialMode {
    Native,
    ProtectedFile,
}

pub struct MachineStore {
    #[cfg(not(target_os = "macos"))]
    paths: Paths,
    #[cfg(not(target_os = "macos"))]
    mode: Option<CredentialMode>,
    #[cfg(not(target_os = "macos"))]
    cache: Mutex<Option<(SystemTime, u64, Credentials)>>,
}

pub async fn delete(paths: &Paths) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let _ = paths;
        tokio::task::spawn_blocking(macos::delete).await?
    }
    #[cfg(not(target_os = "macos"))]
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
        anyhow::ensure!(
            errors.is_empty(),
            "系统凭据删除未完成：{}",
            errors.join("；")
        );
        Ok(())
    }
}

impl MachineStore {
    pub fn new(paths: Paths, mode: Option<CredentialMode>) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let encrypted = paths.secrets.join("login.cred").try_exists()?;
            let plain = paths.secrets.join("login.json").try_exists()?;
            anyhow::ensure!(
                !(encrypted && plain),
                "存在两份不同存储方式的系统凭据，请先清理旧凭据"
            );
            anyhow::ensure!(
                !matches!(mode, Some(CredentialMode::ProtectedFile)) || !encrypted,
                "已有系统凭据使用加密存储，请继续使用 native；不会自动降级为明文"
            );
            anyhow::ensure!(
                !matches!(mode, Some(CredentialMode::Native)) || !plain,
                "已有系统凭据使用 protected-file；切换前请备份并移除旧系统凭据，然后重新 install"
            );
        }
        #[cfg(not(target_os = "linux"))]
        anyhow::ensure!(
            !matches!(mode, Some(CredentialMode::ProtectedFile)),
            "protected-file 仅适用于 Linux；当前平台使用原生机器凭据存储"
        );
        #[cfg(target_os = "macos")]
        let _ = paths.secrets;
        Ok(Self {
            #[cfg(not(target_os = "macos"))]
            paths,
            #[cfg(not(target_os = "macos"))]
            mode,
            #[cfg(not(target_os = "macos"))]
            cache: Mutex::new(None),
        })
    }

    #[cfg(not(target_os = "macos"))]
    fn native_file(&self) -> std::path::PathBuf {
        self.paths.secrets.join("login.cred")
    }
    #[cfg(not(target_os = "macos"))]
    fn plain_file(&self) -> std::path::PathBuf {
        self.paths.secrets.join("login.json")
    }

    pub async fn load(&self, background: bool) -> Result<Option<Credentials>> {
        #[cfg(target_os = "macos")]
        {
            let _ = background;
            tokio::task::spawn_blocking(macos::load).await?
        }
        #[cfg(not(target_os = "macos"))]
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
                && let Some((old, length, credentials)) = &*self
                    .cache
                    .lock()
                    .map_err(|_| anyhow::anyhow!("凭据缓存损坏"))?
                && *old == modified
                && *length == metadata.len()
            {
                return Ok(Some(credentials.clone()));
            }
            #[cfg(target_os = "linux")]
            let bytes = if path == self.plain_file() {
                Zeroizing::new(fs::read(&path)?)
            } else {
                let first_load = self
                    .cache
                    .lock()
                    .map_err(|_| anyhow::anyhow!("凭据缓存损坏"))?
                    .is_none();
                let supplied = if background && first_load {
                    std::env::var_os("CREDENTIALS_DIRECTORY")
                        .map(std::path::PathBuf::from)
                        .map(|dir| dir.join("login"))
                } else {
                    None
                };
                if let Some(supplied) = supplied {
                    validate_private(&supplied)?;
                    Zeroizing::new(fs::read(supplied)?)
                } else {
                    let encrypted = Zeroizing::new(fs::read(&path)?);
                    crypt("decrypt", &encrypted).await?
                }
            };
            #[cfg(windows)]
            let bytes = {
                let _ = background;
                anyhow::ensure!(path == self.native_file(), "Windows 不支持明文机器凭据");
                windows::decrypt(&fs::read(&path)?)?
            };
            let credentials = decode(std::str::from_utf8(&bytes).context("系统凭据不是 UTF-8")?)?;
            *self
                .cache
                .lock()
                .map_err(|_| anyhow::anyhow!("凭据缓存损坏"))? =
                Some((modified, metadata.len(), credentials.clone()));
            Ok(Some(credentials))
        }
    }

    pub async fn save(&self, credentials: Credentials) -> Result<()> {
        let text = encode(&credentials)?;
        #[cfg(target_os = "macos")]
        {
            tokio::task::spawn_blocking(move || macos::save(&text)).await?
        }
        #[cfg(not(target_os = "macos"))]
        {
            private_dir(&self.paths.secrets)?;
            #[cfg(target_os = "linux")]
            let plain = match self.mode {
                Some(CredentialMode::ProtectedFile) => true,
                Some(CredentialMode::Native) => false,
                None => self.plain_file().try_exists()? && !self.native_file().try_exists()?,
            };
            #[cfg(windows)]
            let plain = {
                let _ = &self.mode;
                false
            };
            let (path, bytes) = if plain {
                tracing::warn!("系统凭据将以受文件权限保护的明文保存，仅允许管理员读取。");
                (self.plain_file(), Zeroizing::new(text.as_bytes().to_vec()))
            } else {
                #[cfg(target_os = "linux")]
                let bytes = crypt("encrypt", text.as_bytes()).await?;
                #[cfg(windows)]
                let bytes = windows::encrypt(text.as_bytes())?;
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
                .map_err(|_| anyhow::anyhow!("凭据缓存损坏"))? = None;
            Ok(())
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn validate_private(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "凭据路径必须是普通文件"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        anyhow::ensure!(
            metadata.uid() == 0 && metadata.mode() & 0o077 == 0,
            "系统凭据必须由 root 所有且权限为 0600"
        );
    }
    Ok(())
}

#[cfg(all(test, not(target_os = "macos")))]
mod delete_tests {
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

    #[cfg(unix)]
    #[tokio::test]
    async fn deletion_unlinks_leaf_symlinks_but_rejects_redirected_directories() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut paths = Paths::test(dir.path().join("cxxy-wlanlogin/secrets"));
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
}
