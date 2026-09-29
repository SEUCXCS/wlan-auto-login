//! Terminal input and presentation around the shared login controller.
use super::{LoginArgs, output};
use crate::app::{
    context::Context,
    controller::{
        install,
        login::{self, LoginRequest},
    },
    tui,
};
use crate::infra::{
    error::{Context as _, Error, Message, Result, bail},
    scope::Scope,
};
use std::io::{self, IsTerminal};

pub async fn run(context: &Context, mut args: LoginArgs, installing: bool) -> Result<()> {
    if installing {
        install::validate_scope(&context.paths)?;
    } else {
        login::validate_scope(&context.paths)?;
    }
    let refresh = args.interactive || args.password_stdin;
    if args.password_stdin {
        let mut password = String::new();
        io::stdin()
            .read_line(&mut password)
            .context(Message::FailedToReadPasswordFromStdin)?;
        if password.ends_with('\n') {
            password.pop();
            if password.ends_with('\r') {
                password.pop();
            }
        }
        args.password = Some(password);
    }
    let interactive_requested = args.interactive;
    let non_interactive = args.non_interactive;
    let request = LoginRequest {
        username: args.username,
        password: args.password,
        line: args.line,
        remember_me: args.remember_me,
        credential_store: args.credential_store,
    };
    let wifi = context.wifi().await;
    let store = context.credentials(request.credential_store)?;
    let needs_login = !installing || install::prepare(&request, refresh, &wifi, &store).await?;
    if needs_login {
        let mut prepared =
            login::prepare(&context.paths, request, installing, &wifi, &store).await?;
        let interactive = wants_tui(interactive_requested, &prepared.form);
        if interactive
            && (non_interactive || !io::stdin().is_terminal() || !io::stdout().is_terminal())
        {
            if let Some(warning) = prepared.warning {
                return Err(Error::message(warning)).context(Message::CredentialsRequired);
            }
            bail!(Message::CredentialsRequired);
        }
        loop {
            if interactive {
                let remember_reason = if installing && context.paths.scope == Scope::System {
                    Some("安装系统服务必须保存凭据，供本机开机自动登录使用。")
                } else if installing {
                    Some("安装服务必须保存账号和密码。")
                } else if prepared.save_only {
                    Some("离线配置仅保存账号和密码。")
                } else {
                    None
                };
                prepared.form =
                    tui::prompt(prepared.form, remember_reason, prepared.warning.take())?;
            }
            match login::execute(
                &context.client,
                &context.paths,
                &prepared.form,
                &wifi,
                &store,
            )
            .await
            {
                Ok(result) => {
                    output::login(&result);
                    break;
                }
                Err(error @ Error::Portal(_)) if interactive => {
                    prepared.warning = Some(error.to_string());
                }
                Err(error) => return Err(error),
            }
        }
    }
    if installing {
        install::run(&context.paths).await?;
    }
    Ok(())
}

fn wants_tui(requested: bool, form: &login::Form) -> bool {
    requested || form.credentials.username.trim().is_empty() || form.credentials.password.is_empty()
}
#[cfg(test)]
mod tests {
    use super::*;
    use wlanlogin_lib::{Credentials, Line};
    #[test]
    fn saved_credentials_skip_tui_unless_requested() {
        let saved = Credentials {
            username: "student".into(),
            password: "secret".into(),
        };
        let mut args = LoginRequest::default();
        let form = login::resolve(&mut args, Some(&saved), Line::Free1, false);
        assert!(!wants_tui(false, &form));
        assert!(wants_tui(true, &form));
    }
}
