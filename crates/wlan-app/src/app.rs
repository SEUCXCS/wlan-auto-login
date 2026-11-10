//! Parse arguments and delegate to the selected platform bootstrap.
mod bootstrap;
#[cfg(target_os = "linux")]
#[path = "app/bootstrap/linux/cli.rs"]
pub(crate) mod cli;
#[cfg(target_os = "macos")]
#[path = "app/bootstrap/macos/cli.rs"]
pub(crate) mod cli;
#[cfg(target_os = "windows")]
#[path = "app/bootstrap/windows/cli.rs"]
pub(crate) mod cli;
mod context;
mod controller;
mod daemon;
mod tui;

use clap::Parser;

#[cfg(target_os = "linux")]
use bootstrap::linux::boot as bootstrap;
#[cfg(target_os = "macos")]
use bootstrap::macos::boot as bootstrap;
#[cfg(target_os = "windows")]
use bootstrap::windows::boot as bootstrap;

pub(crate) fn run() -> std::process::ExitCode {
    let args = cli::Cli::parse();
    bootstrap(args)
}

#[cfg(test)]
mod tests {
    use super::cli::{Cli, Command, LogsArgs, ServiceCommand};
    use clap::{CommandFactory, Parser};

    #[test]
    fn service_commands_keep_scope_and_cleanup_options() {
        assert!(matches!(
            Cli::try_parse_from(["app", "daemon"]),
            Ok(Cli {
                command: Command::Daemon { .. },
                ..
            })
        ));
        for scope in ["--user", "--system"] {
            for action in [
                "install",
                "uninstall",
                "start",
                "stop",
                "restart",
                "status",
                "logs",
                "run",
            ] {
                assert!(Cli::try_parse_from(["app", scope, "service", action]).is_ok());
                assert!(Cli::try_parse_from(["app", "service", action, scope]).is_ok());
            }
            assert!(matches!(
                Cli::try_parse_from(["app", scope, "service", "uninstall", "--purge"]),
                Ok(Cli {
                    command: Command::Service(ServiceCommand::Uninstall { purge: true }),
                    ..
                })
            ));
            assert!(matches!(
                Cli::try_parse_from(["app", "logout", "--forget", scope]),
                Ok(Cli {
                    command: Command::Logout { forget: true },
                    ..
                })
            ));
        }
        assert!(Cli::try_parse_from(["app", "--user", "--system", "service", "install"]).is_err());
        assert!(Cli::try_parse_from(["app", "service", "uninstall", "--forget"]).is_err());
        assert!(Cli::try_parse_from(["app", "logout", "--purge"]).is_err());
    }

    #[test]
    fn logs_parse_without_starting_a_service() {
        assert!(matches!(
            Cli::try_parse_from(["app", "--system", "service", "logs", "-n", "0", "-f"]),
            Ok(Cli {
                command: Command::Service(ServiceCommand::Logs(LogsArgs {
                    lines: 0,
                    follow: true
                })),
                ..
            })
        ));
        assert!(Cli::try_parse_from(["app", "service", "logs", "-n", "-1"]).is_err());
        assert!(Cli::try_parse_from(["app", "service", "--unknown-option", "logs"]).is_err());
    }

    #[test]
    fn public_help_exposes_service_and_hides_process_entries() {
        let mut command = Cli::command();
        let help = command.render_help().to_string();
        assert!(help.contains("service"));
        let daemon = command.find_subcommand_mut("daemon");
        assert!(daemon.is_some());
        if let Some(daemon) = daemon {
            let help = daemon.render_help().to_string();
            assert!(!help.contains("--internal-wifi-host"));
            assert!(!help.contains("--system-helper"));
        }
        let service = command.find_subcommand_mut("service");
        assert!(service.is_some());
        if let Some(service) = service {
            let help = service.render_help().to_string();
            assert!(help.contains("install") && help.contains("status"));
            assert!(!help.contains("native") && !help.contains("  run"));
        }
        assert!(help.contains("permission"));
    }

    #[test]
    fn permission_arguments_are_available_on_every_platform() {
        for args in [
            vec!["app", "permission", "list"],
            vec!["app", "permission", "list", "--json"],
            vec!["app", "permission", "check"],
            vec!["app", "permission", "check", "--json"],
            vec![
                "app",
                "--system",
                "permission",
                "check",
                "macos.keychain",
                "--json",
            ],
            vec!["app", "permission", "get", "macos.location"],
            vec!["app", "permission", "get", "custom.permission", "--json"],
        ] {
            assert!(Cli::try_parse_from(args).is_ok());
        }
        assert!(Cli::try_parse_from(["app", "permission", "get"]).is_err());
    }

    #[test]
    fn explicit_tui_and_non_interactive_are_mutually_exclusive() {
        for command in [vec!["login"], vec!["service", "install"]] {
            let mut args = vec!["app"];
            args.extend(command);
            args.extend(["--interactive", "--non-interactive"]);
            assert!(Cli::try_parse_from(args).is_err());
        }
    }
}
