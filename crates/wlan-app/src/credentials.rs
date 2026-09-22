use anyhow::{Context, Result, bail};
use cxxy_wlanlogin_lib::Credentials;
use serde::{Deserialize, Serialize};
use std::{future::Future, sync::Arc, time::Duration};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const SERVICE: &str = "org.seucxcs.cxxy-wlanlogin";
const ENTRY: &str = "saved-login";

pub trait Store {
    fn load(&self) -> impl std::future::Future<Output = Result<Option<Credentials>>>;
    fn save(&self, credentials: Credentials) -> impl std::future::Future<Output = Result<()>>;
}

mod machine;
pub use machine::CredentialMode;
#[cfg(target_os = "macos")]
pub use machine::authorize_binary;
#[cfg(target_os = "linux")]
pub use machine::linux_credential_directive;

pub struct ScopedStore {
    machine: Option<machine::MachineStore>,
}

/// Keep a single read in flight if a native keychain call stops responding.
/// Timing out a JoinHandle must not detach it and start another read every tick.
pub struct BackgroundReader {
    store: Arc<ScopedStore>,
    pending: Option<tokio::task::JoinHandle<Result<Option<Credentials>>>>,
}

impl BackgroundReader {
    pub fn new(paths: &crate::config::Paths) -> Result<Self> {
        Ok(Self {
            store: Arc::new(ScopedStore::new(paths, None)?),
            pending: None,
        })
    }

    pub async fn load(&mut self) -> Result<Option<Credentials>> {
        let store = self.store.clone();
        self.load_with(Duration::from_secs(10), async move {
            store.load_background().await
        })
        .await
    }

    async fn load_with(
        &mut self,
        timeout: Duration,
        read: impl Future<Output = Result<Option<Credentials>>> + Send + 'static,
    ) -> Result<Option<Credentials>> {
        let task = self.pending.get_or_insert_with(|| tokio::spawn(read));
        let result = tokio::time::timeout(timeout, task)
            .await
            .context("后台凭据读取超时，等待当前读取完成后重试")?;
        self.pending = None;
        result.context("后台凭据读取任务失败")?
    }
}

impl Drop for BackgroundReader {
    fn drop(&mut self) {
        if let Some(task) = &self.pending {
            task.abort();
        }
    }
}

impl ScopedStore {
    pub fn new(paths: &crate::config::Paths, mode: Option<CredentialMode>) -> Result<Self> {
        if paths.scope == crate::scope::Scope::System {
            Ok(Self {
                machine: Some(machine::MachineStore::new(paths.clone(), mode)?),
            })
        } else {
            anyhow::ensure!(mode.is_none(), "--credential-store 只适用于系统凭据");
            Ok(Self { machine: None })
        }
    }

    pub async fn load_background(&self) -> Result<Option<Credentials>> {
        match &self.machine {
            Some(store) => store.load(true).await,
            None => load_for_daemon().await,
        }
    }
}

impl Store for ScopedStore {
    async fn load(&self) -> Result<Option<Credentials>> {
        match &self.machine {
            Some(store) => store.load(false).await,
            None => load().await,
        }
    }

    async fn save(&self, credentials: Credentials) -> Result<()> {
        match &self.machine {
            Some(store) => store.save(credentials).await,
            None => save(credentials).await,
        }
    }
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
struct Secret {
    version: u8,
    username: String,
    password: String,
}

fn encode(credentials: &Credentials) -> Result<Zeroizing<String>> {
    anyhow::ensure!(
        !credentials.username.trim().is_empty() && !credentials.password.is_empty(),
        "账号和密码不能为空"
    );
    Ok(Zeroizing::new(serde_json::to_string(&Secret {
        version: 1,
        username: credentials.username.clone(),
        password: credentials.password.clone(),
    })?))
}

pub async fn load() -> Result<Option<Credentials>> {
    tokio::task::spawn_blocking(|| {
        let entry = keyring::Entry::new(SERVICE, ENTRY).context("无法打开系统密钥管理器")?;
        let text = match entry.get_password() {
            Ok(value) => Zeroizing::new(value),
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(e) => return Err(e).context("无法读取凭据，请解锁系统密钥管理器"),
        };
        decode(&text).map(Some)
    })
    .await
    .context("密钥读取任务失败")?
}

/// Background reads never ask the user to unlock the keychain.
pub async fn load_for_daemon() -> Result<Option<Credentials>> {
    #[cfg(target_os = "linux")]
    {
        tokio::task::spawn_blocking(|| {
            use dbus_secret_service::{EncryptionType, SecretService};
            let service = SecretService::connect_with_max_prompt_timeout(EncryptionType::Dh, 0)?;
            let items = service.search_items(std::collections::HashMap::from([
                ("service", SERVICE),
                ("username", ENTRY),
                ("target", "default"),
            ]))?;
            if !items.locked.is_empty() {
                bail!("密钥库尚未解锁，稍后重试");
            }
            if items.unlocked.len() > 1 {
                bail!("找到多份已保存凭据，请在系统密钥管理器中清理重复项");
            }
            match items.unlocked.first() {
                Some(item) => {
                    let bytes = Zeroizing::new(item.get_secret()?);
                    decode(std::str::from_utf8(&bytes).context("保存的凭据不是 UTF-8")?).map(Some)
                }
                None => Ok(None),
            }
        })
        .await
        .context("后台密钥读取任务失败")?
    }
    #[cfg(not(target_os = "linux"))]
    load().await
}

pub fn prepare_daemon() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        // SAFETY: daemon is a separate process and performs no interactive
        // keychain operations. This process-wide switch prevents native dialogs.
        let status =
            unsafe { security_framework_sys::keychain::SecKeychainSetUserInteractionAllowed(0) };
        if status != 0 {
            bail!("无法禁用后台密钥弹窗: {status}");
        }
    }
    Ok(())
}

fn decode(text: &str) -> Result<Credentials> {
    let secret: Secret = serde_json::from_str(text).context("保存的凭据格式无效")?;
    if secret.version != 1 || secret.username.trim().is_empty() || secret.password.is_empty() {
        bail!("保存的凭据不完整，请重新执行 login --remember-me");
    }
    Ok(Credentials {
        username: secret.username.clone(),
        password: secret.password.clone(),
    })
}

pub async fn save(credentials: Credentials) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let text = encode(&credentials)?;
        keyring::Entry::new(SERVICE, ENTRY)
            .context("无法打开系统密钥管理器")?
            .set_password(&text)
            .context("无法保存凭据，请解锁系统密钥管理器")?;
        Ok(())
    })
    .await
    .context("密钥保存任务失败")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn timed_out_background_read_is_reused_until_it_finishes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut reader = BackgroundReader::new(&crate::config::Paths::test(dir.path()))?;
        let release = Arc::new(tokio::sync::Notify::new());
        let wait = release.clone();
        assert!(
            reader
                .load_with(Duration::from_millis(20), async move {
                    wait.notified().await;
                    Ok(Some(Credentials {
                        username: "original".into(),
                        password: "test".into(),
                    }))
                })
                .await
                .is_err()
        );
        for _ in 0..3 {
            assert!(
                reader
                    .load_with(Duration::from_millis(10), async {
                        panic!("a stalled read must not start a second keychain request");
                    })
                    .await
                    .is_err()
            );
        }
        release.notify_one();
        let result = reader
            .load_with(Duration::from_secs(1), async {
                panic!("the original result must be consumed first");
            })
            .await?
            .context("saved credentials expected")?;
        assert_eq!(result.username, "original");
        assert!(
            reader
                .load_with(Duration::from_secs(1), async { Ok(None) })
                .await?
                .is_none()
        );
        Ok(())
    }
    #[test]
    fn rejects_incomplete_or_unknown_saved_credentials() {
        assert!(decode(r#"{"version":1,"username":"student","password":""}"#).is_err());
        assert!(decode(r#"{"version":2,"username":"student","password":"secret"}"#).is_err());
        assert!(decode("not json").is_err());
    }
}
