//! Prepare saved credentials before installing the operating-system service.
use super::{cli::LoginArgs, login};
use crate::{config::Paths, credentials, service, wifi};
use anyhow::Result;
use cxxy_wlanlogin_lib::Client;

pub(super) async fn run(client: &Client, paths: &Paths, args: LoginArgs) -> Result<()> {
    service::ensure_no_conflicting_install(paths)?;
    let wifi = wifi::WifiProbe::new().await;
    if let Some(notice) = wifi::wifi_state(&wifi).await.notice() {
        tracing::warn!("{notice} 后台服务仍可安装，接入 Student_CX 后再自动登录。");
    }
    let store = credentials::ScopedStore::new(paths, args.credential_store)?;
    let saved = credentials::Store::load(&store).await?;
    if saved.is_none()
        || args.username.is_some()
        || args.password.is_some()
        || args.password_stdin
        || args.interactive
        || args.credential_store.is_some()
    {
        tracing::warn!("安装后台服务需要保存账号和密码；本次登录的 Remember me 已强制开启。");
        login::run_with_wifi(client, paths, args, true, &wifi).await?;
    }
    service::install(paths).await
}
