use crate::{
    cli::LoginArgs,
    config::{Paths, Preferences},
    credentials, tui,
};
use anyhow::{Context, Result, bail};
use cxxy_wlanlogin_lib::{Client, Credentials, Line};
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
    mut args: LoginArgs,
    force_remember: bool,
) -> Result<()> {
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
    let mut warning = None;
    let saved = if args.username.is_none() || args.password.is_none() {
        match credentials::load().await {
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
    let mut form = resolve(&mut args, saved.as_ref(), prefs.line, force_remember);
    let mut interactive =
        form.credentials.username.trim().is_empty() || form.credentials.password.is_empty();
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
            form = tui::prompt(form, force_remember, warning.take())?;
        }
        let _operation = crate::control::operation_lock(paths).await?;
        match client.login(&form.credentials, form.line).await {
            Ok(()) => {
                if form.remember {
                    credentials::save(form.credentials.clone())
                        .await
                        .context("已登录，但保存凭据失败；后台服务尚不能安装")?;
                }
                Preferences { line: form.line }.save(paths)?;
                let mut state = crate::control::State::load(paths)?;
                state.resume();
                state.save(paths)?;
                crate::control::wake(paths).await;
                println!(
                    "登录成功（{}）{}",
                    form.line.label(),
                    if form.remember {
                        "，账号和密码已保存"
                    } else {
                        ""
                    }
                );
                return Ok(());
            }
            Err(e) if interactive => {
                warning = Some(e.to_string());
                interactive = true;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changing_account_does_not_reuse_another_accounts_password() {
        let saved = Credentials {
            username: "old".into(),
            password: "old-secret".into(),
        };
        let mut args = LoginArgs {
            username: Some("new".into()),
            ..Default::default()
        };
        let form = resolve(&mut args, Some(&saved), Line::Free2, false);
        assert!(form.credentials.password.is_empty());
        assert_eq!(form.line, Line::Free2);
        assert!(!form.remember);
    }
    #[test]
    fn install_forces_remember_even_without_the_flag() {
        let form = resolve(&mut LoginArgs::default(), None, Line::Free1, true);
        assert!(form.remember);
    }
    #[test]
    fn explicit_credentials_override_saved_values() {
        let saved = Credentials {
            username: "old".into(),
            password: "old-secret".into(),
        };
        let mut args = LoginArgs {
            username: Some("new".into()),
            password: Some("new-secret".into()),
            line: Some(Line::Campus),
            ..Default::default()
        };
        let form = resolve(&mut args, Some(&saved), Line::Free1, false);
        assert_eq!(form.credentials.password, "new-secret");
        assert_eq!(form.line, Line::Campus);
    }
}
