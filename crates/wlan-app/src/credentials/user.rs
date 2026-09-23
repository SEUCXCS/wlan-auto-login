use super::{decode, encode};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use anyhow::bail;
use anyhow::{Context, Result};
use cxxy_wlanlogin_lib::Credentials;
use zeroize::Zeroizing;

const SERVICE: &str = "org.seucxcs.cxxy-wlanlogin";
const ENTRY: &str = "saved-login";

pub async fn delete() -> Result<()> {
    tokio::task::spawn_blocking(|| {
        #[cfg(target_os = "linux")]
        {
            use dbus_secret_service::{EncryptionType, SecretService};
            let service = SecretService::connect(EncryptionType::Dh)?;
            let items = service.search_items(std::collections::HashMap::from([
                ("service", SERVICE),
                ("username", ENTRY),
                ("target", "default"),
            ]))?;
            // Remove every matching application item, including duplicates, without
            // reading secrets or touching the collection or other applications.
            for item in items.unlocked.iter().chain(&items.locked) {
                item.delete()
                    .context("无法删除用户凭据，请解锁系统密钥管理器")?;
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        match keyring::Entry::new(SERVICE, ENTRY)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error).context("无法删除用户凭据"),
        }
    })
    .await
    .context("凭据删除任务失败")?
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
