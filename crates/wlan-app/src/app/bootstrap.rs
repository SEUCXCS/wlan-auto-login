//! Initialize shared application state and compose platform startup.
#[cfg(target_os = "linux")]
pub(super) mod linux;
#[cfg(target_os = "macos")]
pub(super) mod macos;
#[cfg(target_os = "windows")]
pub(super) mod windows;

use super::{cli, context};
use crate::infra::{config::Config, error::Message, logging, paths::Paths, scope, shutdown};
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
            let _ = logging::init(None, None, Default::default());
            tracing::error!(error = %error, "{}", Message::RuntimeCreationFailed);
            ExitCode::FAILURE
        }
    }
}

async fn entry(args: cli::Cli) -> ExitCode {
    let selected =
        scope::elevated().map(|elevated| scope::Scope::select(args.user, args.system, elevated));
    let paths = selected.and_then(Paths::new);
    platform::configure(&args, &paths);
    let scope = paths.as_ref().ok().map(|paths| paths.scope);
    let level = paths
        .as_ref()
        .ok()
        .map(|paths| Config::load(paths).and_then(|config| config.log_level()))
        .transpose();
    let default_level = level
        .as_ref()
        .ok()
        .and_then(|value| *value)
        .unwrap_or_default();

    let log = if matches!(
        args.command,
        cli::Command::Service(cli::ServiceCommand::Run)
    ) {
        paths
            .as_ref()
            .map_err(|error| crate::infra::error::error!("{error:#}"))
            .and_then(logging::background_log)
            .map(Some)
    } else {
        Ok(None)
    };
    let result = match log.and_then(|file| logging::init(file, scope, default_level)) {
        Ok(()) => {
            if let Err(error) = level {
                tracing::warn!(target: "wlanlogin::config", error = %error, "Using the default logging level");
            }
            match paths {
                Ok(paths) => dispatch(args, paths).await,
                Err(error) => Err(error),
            }
        }
        Err(error) => {
            // stderr logging remains available if opening the background log failed.
            let _ = logging::init(None, scope, default_level);
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

async fn dispatch(args: cli::Cli, paths: Paths) -> crate::infra::error::Result<()> {
    let cli::Cli {
        user,
        system,
        command,
    } = args;

    let command = match command {
        cli::Command::Config(command) => return cli::config::run(command, &paths).await,
        cli::Command::Permission(command) => {
            if !matches!(command, cli::PermissionCommand::List { .. }) {
                paths.scope.require_access()?;
            }
            return cli::permission::run(command, paths.scope).await;
        }
        command => command,
    };
    let Some(command) = platform::dispatch(command, &paths).await? else {
        return Ok(());
    };
    let args = cli::Cli {
        user,
        system,
        command,
    };
    let context = context::Context::new(paths)?;
    if matches!(
        args.command,
        cli::Command::Service(cli::ServiceCommand::Run)
    ) {
        context.paths.scope.require_access()?;
        return crate::app::controller::daemon::run(&context, true).await;
    }
    cli::dispatch(args, &context).await
}

#[cfg(target_os = "linux")]
pub(super) use linux as platform;
#[cfg(target_os = "macos")]
pub(super) use macos as platform;
#[cfg(target_os = "windows")]
pub(super) use windows as platform;
