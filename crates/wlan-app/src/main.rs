mod cli;
mod config;
mod control;
mod credentials;
mod daemon;
mod logging;
mod login;
mod platform;
mod scope;
mod service;
mod transport;
mod tui;
#[cfg(windows)]
mod windows_service;

use anyhow::Result;
use clap::Parser;
use cxxy_wlanlogin_lib::{Client, PortalConfig};

fn main() -> std::process::ExitCode {
    let args = cli::Cli::parse();
    #[cfg(windows)]
    if matches!(
        args.command,
        cli::Command::Daemon {
            service: true,
            command: None,
            ..
        }
    ) {
        return windows_service::dispatch();
    }
    match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime.block_on(entry(args)),
        Err(error) => {
            let _ = logging::init(None);
            tracing::error!("无法创建运行时：{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn entry(args: cli::Cli) -> std::process::ExitCode {
    let selected =
        scope::elevated().map(|elevated| scope::Scope::select(args.user, args.system, elevated));
    let paths = selected.and_then(config::Paths::new);
    let log = if matches!(
        args.command,
        cli::Command::Daemon {
            command: None,
            background: true,
            ..
        }
    ) {
        paths
            .as_ref()
            .map_err(|error| anyhow::anyhow!("{error:#}"))
            .and_then(service::background_log)
            .map(Some)
    } else {
        Ok(None)
    };
    let result = match log.and_then(logging::init) {
        Ok(()) => match paths {
            Ok(paths) => run(args, paths).await,
            Err(error) => Err(error),
        },
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

async fn run(args: cli::Cli, paths: config::Paths) -> Result<()> {
    if !matches!(
        args.command,
        cli::Command::Daemon {
            command: Some(cli::DaemonCommand::Status),
            ..
        }
    ) {
        paths.scope.require_access()?;
    }
    let client = Client::new(PortalConfig::default())?;
    match args.command {
        cli::Command::Daemon {
            command: Some(command),
            ..
        } => match command {
            cli::DaemonCommand::Status => service::status(&paths, !args.user && !args.system).await,
            cli::DaemonCommand::Start => service::manage(&paths, service::Action::Start).await,
            cli::DaemonCommand::Stop => service::manage(&paths, service::Action::Stop).await,
            cli::DaemonCommand::Restart => service::manage(&paths, service::Action::Restart).await,
        },
        cli::Command::Daemon {
            command: None,
            background,
            service,
        } => {
            #[cfg(not(windows))]
            anyhow::ensure!(!service, "--service 仅用于 Windows Service Control Manager");
            #[cfg(windows)]
            let _ = service;
            if background {
                #[cfg(windows)]
                unsafe {
                    windows_sys::Win32::System::Console::FreeConsole();
                }
            }
            daemon::run(&client, &paths).await
        }
        cli::Command::Install(args) => {
            service::ensure_no_conflicting_install(&paths)?;
            let wifi = platform::WifiProbe::new().await;
            if let Some(notice) = platform::wifi_state(&wifi).await.notice() {
                tracing::warn!("{notice} 后台服务仍可安装，接入 Student_CX 后再自动登录。");
            }
            let store = credentials::ScopedStore::new(&paths, args.credential_store)?;
            let saved = credentials::Store::load(&store).await?;
            if saved.is_none()
                || args.username.is_some()
                || args.password.is_some()
                || args.password_stdin
                || args.interactive
                || args.credential_store.is_some()
            {
                tracing::warn!(
                    "安装后台服务需要保存账号和密码；本次登录的 Remember me 已强制开启。"
                );
                login::run_with_wifi(&client, &paths, args, true, &wifi).await?;
            }
            service::install(&paths).await
        }
        cli::Command::Uninstall => service::uninstall(&paths).await,
        cli::Command::Login(args) => {
            service::ensure_no_conflicting_install(&paths)?;
            login::run(&client, &paths, args, false).await
        }
        cli::Command::Logout => {
            service::ensure_no_conflicting_install(&paths)?;
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
