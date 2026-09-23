//! Initialize the application and route commands to their workflows.
mod cli;
mod daemon;
mod install;
mod login;
mod logout;

pub(crate) use login::Form;

use crate::{config, logging, scope, service, shutdown};
use anyhow::Result;
use clap::Parser;
use cxxy_wlanlogin_lib::{Client, PortalConfig};

pub(crate) fn run() -> std::process::ExitCode {
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
        return service::windows::dispatch();
    }
    match tokio::runtime::Runtime::new() {
        Ok(runtime) => {
            let result = runtime.block_on(entry(args));
            runtime.shutdown_timeout(shutdown::RUNTIME_GRACE);
            result
        }
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
            .and_then(logging::background_log)
            .map(Some)
    } else {
        Ok(None)
    };
    let result = match log.and_then(logging::init) {
        Ok(()) => match paths {
            Ok(paths) => dispatch(args, paths).await,
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

async fn dispatch(args: cli::Cli, paths: config::Paths) -> Result<()> {
    if !matches!(
        args.command,
        cli::Command::Daemon {
            command: Some(cli::DaemonCommand::Status | cli::DaemonCommand::Logs(_)),
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
            cli::DaemonCommand::Logs(options) => {
                service::logs(
                    &paths,
                    service::LogOptions {
                        lines: options.lines,
                        follow: options.follow,
                    },
                )
                .await
            }
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
        cli::Command::Install(args) => install::run(&client, &paths, args).await,
        cli::Command::Uninstall { purge } => service::uninstall(&paths, purge).await,
        cli::Command::Login(args) => {
            service::ensure_no_conflicting_install(&paths)?;
            login::run(&client, &paths, args, false).await
        }
        cli::Command::Logout { forget } => logout::run(&client, &paths, forget).await,
    }
}

#[cfg(windows)]
pub(crate) async fn run_service() -> std::process::ExitCode {
    entry(cli::Cli {
        user: false,
        system: true,
        command: cli::Command::Daemon {
            command: None,
            background: true,
            service: true,
        },
    })
    .await
}
