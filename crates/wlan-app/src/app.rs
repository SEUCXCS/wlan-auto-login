//! Initialize the application and route commands to their workflows.
use crate::infra::error::Message;
mod cli;
mod context;
mod controller;
mod daemon;
mod tui;

use crate::{infra::logging, infra::paths as config, infra::scope, infra::shutdown};
use clap::Parser;

pub(crate) fn run() -> std::process::ExitCode {
    #[cfg(target_os = "macos")]
    let args = if std::env::args_os().len() == 1 && crate::system::macos::bundle::current().is_ok()
    {
        cli::Cli::parse_from([crate::infra::identity::APP_NAME, "macos-host"])
    } else {
        cli::Cli::parse()
    };
    #[cfg(not(target_os = "macos"))]
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
        return crate::system::windows::services::runtime::dispatch(|| Box::pin(run_service()));
    }
    match tokio::runtime::Runtime::new() {
        Ok(runtime) => {
            let result = runtime.block_on(entry(args));
            runtime.shutdown_timeout(shutdown::RUNTIME_GRACE);
            result
        }
        Err(error) => {
            let _ = logging::init(None);
            tracing::error!(error = %error, "{}", Message::RuntimeCreationFailed);
            std::process::ExitCode::FAILURE
        }
    }
}

async fn entry(args: cli::Cli) -> std::process::ExitCode {
    let selected =
        scope::elevated().map(|elevated| scope::Scope::select(args.user, args.system, elevated));
    let paths = selected.and_then(config::Paths::new);
    #[cfg(target_os = "macos")]
    if let Ok(paths) = &paths {
        crate::system::macos::host::configure(paths.scope);
        use std::io::IsTerminal;
        let interactive = match &args.command {
            cli::Command::Login(args) | cli::Command::Install(args) => !args.non_interactive,
            cli::Command::Logout { .. } | cli::Command::Uninstall { .. } => true,
            _ => false,
        } && std::io::stderr().is_terminal();
        crate::system::macos::credentials::access::configure_interaction(interactive);
    }
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
            .map_err(|error| crate::infra::error::error!("{error:#}"))
            .and_then(logging::background_log)
            .map(Some)
    } else {
        Ok(None)
    };
    let result = match log.and_then(logging::init) {
        Ok(()) => match paths {
            Ok(paths) => match context::Context::new(paths) {
                Ok(context) => cli::dispatch(args, &context).await,
                Err(error) => Err(error),
            },
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
            tracing::error!(error = %error, "{}", Message::OperationFailed);
            std::process::ExitCode::FAILURE
        }
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
