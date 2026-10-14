//! Select the native SCM entry or the ordinary Tokio command entry.
use super::entry;
use crate::app::cli::{Cli, Command};
use crate::system::windows::services::runtime;
use std::process::ExitCode;

pub(in crate::app) fn boot(args: Cli) -> ExitCode {
    if matches!(
        args.command,
        Command::Daemon {
            service: true,
            command: None,
            ..
        }
    ) {
        return runtime::dispatch(|| Box::pin(run_service()));
    }
    super::run_with_runtime(|runtime| runtime.block_on(entry(args)))
}

async fn run_service() -> ExitCode {
    entry(Cli {
        user: false,
        system: true,
        command: Command::Daemon {
            command: None,
            background: true,
            service: true,
        },
    })
    .await
}
