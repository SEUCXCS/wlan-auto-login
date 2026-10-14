//! Keep AppKit on the main thread for GUI hosts and managed user daemons.
use super::entry;
use crate::app::cli::{Cli, Command};
use crate::system::macos::host;
use std::process::ExitCode;

pub(in crate::app) fn boot(args: Cli) -> ExitCode {
    super::run_with_runtime(|runtime| {
        // SAFETY: geteuid only reads the current process's effective user ID.
        let gui = unsafe { libc::geteuid() } != 0
            && (matches!(args.command, Command::MacosHost { .. })
                || (matches!(
                    args.command,
                    Command::Daemon {
                        command: None,
                        background: true,
                        ..
                    }
                ) && std::env::var_os("WLANLOGIN_MANAGED").is_some_and(|v| v == "1")));
        if gui {
            host::run_gui(runtime, entry(args))
        } else {
            runtime.block_on(entry(args))
        }
    })
}
