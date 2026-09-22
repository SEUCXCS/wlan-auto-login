use anyhow::{Context, Result, bail};
use cxxy_wlanlogin_lib::Credentials;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const SERVICE: &str = "org.seucxcs.cxxy-wlanlogin";
const ENTRY: &str = "saved-login";

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
struct Secret {
    version: u8,
    username: String,
    password: String,
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
        let secret = Secret {
            version: 1,
            username: credentials.username.clone(),
            password: credentials.password.clone(),
        };
        let text = Zeroizing::new(serde_json::to_string(&secret)?);
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
    #[test]
    fn rejects_incomplete_or_unknown_saved_credentials() {
        assert!(decode(r#"{"version":1,"username":"student","password":""}"#).is_err());
        assert!(decode(r#"{"version":2,"username":"student","password":"secret"}"#).is_err());
        assert!(decode("not json").is_err());
    }
}
