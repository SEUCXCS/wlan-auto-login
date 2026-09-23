use super::cli::LoginArgs;
use crate::{
    config::{Paths, Preferences},
    credentials, tui, wifi,
};
use anyhow::{Context, Result, bail};
use cxxy_wlanlogin_lib::{Client, Credentials, Line, LoginOutcome};
use std::io::{self, IsTerminal};

pub struct Form {
    pub credentials: Credentials,
    pub line: Line,
    pub remember: bool,
}

pub fn resolve(
    args: &mut LoginArgs,
    saved: Option<&Credentials>,
    default_line: Line,
    force_remember: bool,
) -> Form {
    let username = args
        .username
        .take()
        .or_else(|| saved.map(|c| c.username.clone()))
        .unwrap_or_default();
    let password = args
        .password
        .take()
        .or_else(|| {
            saved
                .filter(|c| c.username == username)
                .map(|c| c.password.clone())
        })
        .unwrap_or_default();
    Form {
        credentials: Credentials { username, password },
        line: args.line.unwrap_or(default_line),
        remember: force_remember || args.remember_me,
    }
}

pub async fn run(
    client: &Client,
    paths: &Paths,
    args: LoginArgs,
    force_remember: bool,
) -> Result<()> {
    let wifi = wifi::WifiProbe::new().await;
    run_with_wifi(client, paths, args, force_remember, &wifi).await
}

pub async fn run_with_wifi(
    client: &Client,
    paths: &Paths,
    args: LoginArgs,
    force_remember: bool,
    wifi: &impl wifi::WifiConnection,
) -> Result<()> {
    let store = credentials::ScopedStore::new(paths, args.credential_store)?;
    run_with_store(client, paths, args, force_remember, wifi, &store).await
}

async fn run_with_store(
    client: &Client,
    paths: &Paths,
    mut args: LoginArgs,
    force_remember: bool,
    wifi: &impl wifi::WifiConnection,
    store: &impl credentials::Store,
) -> Result<()> {
    let initial_wifi = wifi::wifi_state(wifi).await;
    let save_only = !initial_wifi.is_target();
    if args.password_stdin {
        let mut password = String::new();
        io::stdin()
            .read_line(&mut password)
            .context("无法读取密码")?;
        if password.ends_with('\n') {
            password.pop();
            if password.ends_with('\r') {
                password.pop();
            }
        }
        args.password = Some(password);
    }
    let mut warning = initial_wifi.notice();
    let saved = if args.username.is_none() || args.password.is_none() {
        match store.load().await {
            Ok(value) => value,
            Err(e) => {
                warning = Some(e.to_string());
                None
            }
        }
    } else {
        None
    };
    let prefs = Preferences::load(paths)?;
    let mut form = resolve(
        &mut args,
        saved.as_ref(),
        prefs.line,
        force_remember || save_only,
    );
    let interactive = wants_tui(&args, &form);
    if interactive
        && (args.non_interactive || !io::stdin().is_terminal() || !io::stdout().is_terminal())
    {
        bail!(
            "缺少账号或密码；请通过参数、CXXY_WLAN_USERNAME/CXXY_WLAN_PASSWORD 或交互终端提供凭据{}",
            warning.map(|s| format!("；{s}")).unwrap_or_default()
        );
    }
    loop {
        if interactive {
            let remember_reason = if force_remember && paths.scope == crate::scope::Scope::System {
                Some("安装系统服务必须保存凭据，供本机开机自动登录使用。")
            } else if force_remember {
                Some("安装服务必须保存账号和密码。")
            } else if save_only {
                Some("离线配置仅保存账号和密码。")
            } else {
                None
            };
            form = tui::prompt(form, remember_reason, warning.take())?;
        }
        let _operation = crate::control::operation_lock(paths).await?;
        match submit(client, &form, wifi).await {
            Ok(outcome) => {
                let authenticated = outcome.is_some();
                let remember = form.remember || !authenticated;
                if remember {
                    store
                        .save(form.credentials.clone())
                        .await
                        .context(if authenticated {
                            "已登录，但保存凭据失败；后台服务尚不能安装"
                        } else {
                            "离线保存凭据失败；后台服务尚不能安装"
                        })?;
                }
                Preferences { line: form.line }.save(paths)?;
                let mut state = crate::control::State::load(paths)?;
                state.resume();
                state.save(paths)?;
                crate::control::wake(paths).await;
                if outcome == Some(LoginOutcome::AlreadyOnline) {
                    tracing::info!(
                        "当前设备已在线，未重复提交登录；本次提供的凭据和线路尚未在线验证{}",
                        if remember {
                            "，账号和密码已保存"
                        } else {
                            ""
                        }
                    );
                } else if authenticated {
                    tracing::info!(
                        "登录成功（{}）{}",
                        form.line.label(),
                        if remember {
                            "，账号和密码已保存"
                        } else {
                            ""
                        }
                    );
                } else {
                    tracing::info!("账号和密码已保存；未执行校园网认证，凭据尚未在线验证。");
                }
                return Ok(());
            }
            Err(e) if interactive => {
                warning = Some(format!("{e:#}"));
            }
            Err(e) => return Err(e),
        }
    }
}

async fn submit(
    client: &Client,
    form: &Form,
    wifi: &impl wifi::WifiConnection,
) -> Result<Option<LoginOutcome>> {
    // Credential entry, keychain access and the operation lock may all take time.
    // Re-read the SSID before Client::login sends even its initial status request.
    let state = wifi::wifi_state(wifi).await;
    if !state.is_target() {
        if let Some(notice) = state.notice() {
            tracing::warn!("{notice}");
        }
        return Ok(None);
    }
    Ok(Some(client.login(&form.credentials, form.line).await?))
}

fn wants_tui(args: &LoginArgs, form: &Form) -> bool {
    args.interactive
        || form.credentials.username.trim().is_empty()
        || form.credentials.password.is_empty()
}

#[cfg(test)]
mod tests;
