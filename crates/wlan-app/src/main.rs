mod cli;
mod config;
mod control;
mod credentials;
mod daemon;
mod logging;
mod login;
mod platform;
mod service;
mod transport;
mod tui;

use anyhow::Result;
use clap::Parser;
use cxxy_wlanlogin_lib::{Client, PortalConfig};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = cli::Cli::parse();
    let log = if matches!(
        args.command,
        cli::Command::Daemon {
            command: None,
            background: true
        }
    ) {
        config::Paths::new()
            .and_then(|paths| service::background_log(&paths))
            .map(Some)
    } else {
        Ok(None)
    };
    let result = match log.and_then(logging::init) {
        Ok(()) => run(args).await,
        Err(error) => {
            // stderr logging remains available if opening the background log failed.
            let _ = logging::init(None);
            Err(error)
        }
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run(args: cli::Cli) -> Result<()> {
    let client = Client::new(PortalConfig::default())?;
    match args.command {
        cli::Command::Daemon {
            command: Some(command),
            ..
        } => {
            let paths = config::Paths::new()?;
            match command {
                cli::DaemonCommand::Status => service::status(&paths).await,
                cli::DaemonCommand::Start => service::manage(&paths, service::Action::Start).await,
                cli::DaemonCommand::Stop => service::manage(&paths, service::Action::Stop).await,
                cli::DaemonCommand::Restart => {
                    service::manage(&paths, service::Action::Restart).await
                }
            }
        }
        cli::Command::Daemon {
            command: None,
            background,
        } => {
            let paths = config::Paths::new()?;
            if background {
                #[cfg(windows)]
                unsafe {
                    windows_sys::Win32::System::Console::FreeConsole();
                }
            }
            daemon::run(&client, &paths).await
        }
        cli::Command::Install(args) => {
            let wifi = platform::WifiProbe::new().await;
            if let Some(notice) = platform::wifi_state(&wifi).await.notice() {
                tracing::warn!("{notice} 后台服务仍可安装，接入 Student_CX 后再自动登录。");
            }
            let paths = config::Paths::new()?;
            let saved = credentials::load().await?;
            if saved.is_none()
                || args.username.is_some()
                || args.password.is_some()
                || args.password_stdin
                || args.interactive
            {
                tracing::warn!(
                    "安装后台服务需要保存账号和密码；本次登录的 Remember me 已强制开启。"
                );
                login::run_with_wifi(&client, &paths, args, true, &wifi).await?;
            }
            service::install(&paths).await
        }
        cli::Command::Uninstall => service::uninstall(&config::Paths::new()?).await,
        cli::Command::Login(args) => login::run(&client, &config::Paths::new()?, args, false).await,
        cli::Command::Logout => {
            let paths = config::Paths::new()?;
            let _operation = control::operation_lock(&paths).await?;
            let previous = control::State::load(&paths)?;
            let mut paused = previous.clone();
            paused.paused = true;
            paused.save(&paths)?;
            if let Err(error) = client.logout().await {
                previous.save(&paths)?;
                return Err(error.into());
            }
            control::wake(&paths).await;
            tracing::info!("已注销");
            Ok(())
        }
    }
}
