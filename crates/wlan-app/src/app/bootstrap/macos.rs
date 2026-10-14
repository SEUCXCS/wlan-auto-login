//! Keep AppKit on the main thread for GUI hosts and managed user daemons.
use super::entry;
use crate::app::cli::{Cli, Command, NativeServiceCommand, ServiceCommand};
use crate::system::macos::runtime::host;
use std::process::ExitCode;

pub(in crate::app) fn boot(args: Cli) -> ExitCode {
    super::run_with_runtime(|runtime| {
        // SAFETY: geteuid only reads the current process's effective user ID.
        let gui = unsafe { libc::geteuid() } != 0
            && (matches!(
                args.command,
                Command::Daemon {
                    internal_wifi_host: true,
                    ..
                }
            ) || matches!(args.command, Command::Service(ServiceCommand::Run)));
        if gui {
            crate::system::macos::runtime::gui::run_gui(runtime, entry(args))
        } else {
            runtime.block_on(entry(args))
        }
    })
}

/// Platform-only entries bypass ordinary command dispatch and portal dependencies.
pub(super) async fn dispatch(
    command: Command,
    paths: &crate::infra::paths::Paths,
) -> crate::infra::error::Result<Option<Command>> {
    match command {
        Command::Service(ServiceCommand::Native(action)) => {
            if !matches!(action, NativeServiceCommand::Status) {
                paths.scope.require_access()?;
            }
            let scope = paths.scope;
            let action = match action {
                NativeServiceCommand::Status => "status",
                NativeServiceCommand::Register => "register",
                NativeServiceCommand::Unregister => "unregister",
            };
            let result = tokio::task::spawn_blocking(move || {
                crate::system::macos::services::app_service::perform(scope, action)
            })
            .await??;
            println!("{result}");
        }
        Command::Daemon {
            internal_wifi_host: true,
            system_helper,
            ..
        } => {
            paths.scope.require_access()?;
            let host = host::Host::start(if system_helper {
                host::Role::System
            } else {
                host::Role::Foreground
            })
            .await?;
            let mut shutdown = crate::infra::shutdown::Shutdown::new()?;
            crate::infra::shutdown::until_stopped(host.wait(), shutdown.wait()).await?;
        }
        command => return Ok(Some(command)),
    }
    Ok(None)
}
