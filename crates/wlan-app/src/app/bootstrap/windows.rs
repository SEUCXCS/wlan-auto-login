//! Select the native SCM entry or the ordinary Tokio command entry.
use super::entry;
use crate::app::cli::{Cli, Command, ServiceCommand};
use crate::infra::scope::{self, Scope};
use crate::system::windows::runtime::service as runtime;
use std::process::ExitCode;

pub(in crate::app) fn boot(args: Cli) -> ExitCode {
    let service_entry = matches!(args.command, Command::Service(ServiceCommand::Run));
    if service_entry
        && scope::elevated()
            .is_ok_and(|elevated| Scope::select(args.user, args.system, elevated) == Scope::System)
    {
        return runtime::dispatch(|| Box::pin(run_service()));
    }
    super::run_with_runtime(|runtime| runtime.block_on(entry(args)))
}

async fn run_service() -> ExitCode {
    entry(Cli {
        user: false,
        system: true,
        command: Command::Service(ServiceCommand::Run),
    })
    .await
}
