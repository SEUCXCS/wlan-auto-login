//! Start Linux commands on the shared Tokio runtime.
use super::entry;
use crate::app::cli::Cli;
use std::process::ExitCode;

pub(in crate::app) fn boot(args: Cli) -> ExitCode {
    super::run_with_runtime(|runtime| runtime.block_on(entry(args)))
}

pub(super) fn configure(
    args: &crate::app::cli::Cli,
    paths: &crate::infra::error::Result<crate::infra::paths::Paths>,
) {
    let _ = (args, paths);
}
pub(super) async fn dispatch(
    command: crate::app::cli::Command,
    paths: &crate::infra::paths::Paths,
) -> crate::infra::error::Result<Option<crate::app::cli::Command>> {
    use crate::app::cli;

    match &command {
        cli::Command::Service(cli::ServiceCommand::SessionReady) => {
            crate::system::flow::session::ready(paths).await?;
            return Ok(None);
        }
        cli::Command::Service(cli::ServiceCommand::SessionEnd) => {
            crate::system::flow::session::end(paths).await?;
            return Ok(None);
        }
        _ => {}
    }
    Ok(Some(command))
}
pub(in crate::app) async fn prepare_login(
    context: &crate::app::context::Context,
    installing: bool,
    non_interactive: bool,
) -> crate::infra::error::Result<()> {
    let _ = (context, installing, non_interactive);
    Ok(())
}
