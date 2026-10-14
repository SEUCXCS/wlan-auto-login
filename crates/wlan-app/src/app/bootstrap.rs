//! Initialize shared application state and compose platform startup.
#[cfg(target_os = "linux")]
pub(super) mod linux;
#[cfg(target_os = "macos")]
pub(super) mod macos;
#[cfg(target_os = "windows")]
pub(super) mod windows;

use super::{cli, context};
use crate::infra::{error::Message, logging, paths as config, scope, shutdown};
use std::process::ExitCode;
use tokio::runtime::Runtime;

fn run_with_runtime(run: impl FnOnce(&Runtime) -> ExitCode) -> ExitCode {
    match Runtime::new() {
        Ok(runtime) => {
            let result = run(&runtime);
            runtime.shutdown_timeout(shutdown::RUNTIME_GRACE);
            result
        }
        Err(error) => {
            let _ = logging::init(None);
            tracing::error!(error = %error, "{}", Message::RuntimeCreationFailed);
            ExitCode::FAILURE
        }
    }
}

async fn entry(args: cli::Cli) -> ExitCode {
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
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "{}", Message::OperationFailed);
            ExitCode::FAILURE
        }
    }
}
