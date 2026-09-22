#[cfg(not(target_os = "macos"))]
use super::decode;
use super::encode;
use crate::config::Paths;
#[cfg(not(target_os = "macos"))]
use crate::config::{private_dir, write_private};
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

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
        assert!(validate_private(&secret).is_err());
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o600))?;
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&secret, &link)?;
        assert!(validate_private(&link).is_err());
        Ok(())
    }
}

#[cfg(target_os = "linux")]
async fn crypt(operation: &str, input: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    crypt_with(std::ffi::OsStr::new("systemd-creds"), operation, input).await
}

#[cfg(target_os = "linux")]
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
    let mut child = command.spawn().context("无法运行 systemd-creds；需要支持加密凭据的 systemd，旧系统可显式选择 --credential-store protected-file")?;
    let mut stdin = child.stdin.take().context("无法打开凭据输入通道")?;
    let output = tokio::time::timeout(Duration::from_secs(30), async {
        stdin.write_all(input).await?;
        drop(stdin);
        child.wait_with_output().await
    })
    .await
    .context("系统凭据加解密超时")??;
    anyhow::ensure!(
        output.status.success(),
        "系统凭据{operation}失败；未降级存储方式"
    );
    Ok(Zeroizing::new(output.stdout))
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

#[cfg(target_os = "linux")]
pub fn linux_credential_directive(paths: &Paths) -> Result<String> {
    if paths.secrets.join("login.cred").try_exists()? {
        Ok(format!(
            "LoadCredentialEncrypted=login:{}\n",
            paths.secrets.join("login.cred").display()
        ))
    } else if paths.secrets.join("login.json").try_exists()? {
        // The explicit protected-file backend also supports systemd versions
        // predating LoadCredential; the root service reads its private file.
        Ok(String::new())
    } else {
        anyhow::bail!("缺少系统凭据，请先执行 sudo login --remember-me")
    }
}
