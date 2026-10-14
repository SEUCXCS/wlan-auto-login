#[cfg(target_os = "linux")]
use crate::infra::command::SystemRunner;

use crate::system::shared::services::{
    Manager,
    artifacts::{Artifacts, Plan, plan},
};

use crate::infra::error::Message;
use crate::infra::error::{Result, ensure};
pub(crate) use crate::infra::file_log::LogOptions;
use crate::infra::file_log::file_logs;
use crate::infra::paths::Paths;
#[cfg(target_os = "linux")]
use crate::system::linux::services::logs::{follow_journal, journal_snapshot};

use crate::system::define::services::LogSource as Source;
fn source(paths: &Paths, plan: &Plan, options: LogOptions) -> Result<Source> {
    let files = Artifacts::read(plan)?;
    ensure!(
        files.definition || files.executable,
        Message::ServiceNotInstalled
    );
    ensure!(files.complete(), Message::InstallationIncomplete);
    // Reading diagnostics must also work when the service is stopped or its
    // registration is broken. No service manager query or mutation is needed.
    match &plan.manager {
        #[cfg(any(target_os = "linux", test))]
        Manager::Systemd { .. } => {
            crate::system::linux::services::logs::source(paths, plan, options)
        }
        #[cfg(any(target_os = "macos", test))]
        Manager::Launchd { .. } | Manager::MacosAppService { .. } => {
            crate::system::macos::services::logs::source(paths, plan, options)
        }
        #[cfg(any(target_os = "windows", test))]
        Manager::TaskScheduler { .. } | Manager::WindowsService => {
            crate::system::windows::services::logs::source(paths, plan, options)
        }
    }
}

pub async fn run(
    paths: &Paths,
    options: LogOptions,
    output: &mut impl std::io::Write,
    stop: impl std::future::Future<Output = std::io::Result<()>>,
) -> Result<()> {
    let source = source(paths, &plan(paths)?, options)?;
    let result = match source {
        #[cfg(target_os = "linux")]
        Source::Journal(command) if options.follow => follow_journal(&command, output, stop).await,
        #[cfg(target_os = "linux")]
        Source::Journal(command) => journal_snapshot(&command, &SystemRunner, output).await,
        Source::File(path) => file_logs(&path, options, output, stop).await,
        // Other platform implementations are compiled for unit tests only.
        #[cfg(all(test, not(target_os = "linux")))]
        _ => unreachable!("the native service plan selects the native log source"),
    };
    // Closing a downstream pipe is a normal way to stop consuming logs.
    match result {
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::BrokenPipe) =>
        {
            Ok(())
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::error::Context;
    use crate::infra::scope::Scope;
    use std::fs;

    #[test]
    fn sources_match_each_platform_and_installation_scope() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let options = LogOptions {
            lines: 37,
            follow: true,
        };
        for scope in [Scope::User, Scope::System] {
            let mut paths = Paths::test(dir.path().join(scope.label()));
            paths.scope = scope;
            let mut plan = crate::system::shared::services::artifacts::linux_plan(
                dir.path().join("daemon"),
                dir.path().join("service"),
            )?;
            plan.scope = scope;
            plan.manager = Manager::Systemd {
                system: scope == Scope::System,
            };
            fs::write(&plan.definition, &plan.content)?;
            fs::write(&plan.executable, "mock, never run")?;
            let Source::File(path) = source(&paths, &plan, options)? else {
                crate::infra::error::bail!("new Linux service must use a removable application log")
            };
            assert_eq!(path, paths.config.join("daemon.log"));
            fs::write(
                &plan.definition,
                plan.content.replace("# ApplicationLog=daemon.log\n", ""),
            )?;
            let Source::Journal(command) = source(&paths, &plan, options)? else {
                crate::infra::error::bail!("journal expected")
            };
            let arguments: Vec<_> = command
                .args
                .iter()
                .map(|arg| arg.to_string_lossy())
                .collect();
            assert_eq!(command.program, "journalctl");
            assert_eq!(
                arguments,
                [
                    if scope == Scope::System {
                        "--system"
                    } else {
                        "--user"
                    },
                    "--unit",
                    "dev.cxcs.tools.wlanlogin.service",
                    "--no-pager",
                    "--output=cat",
                    "--quiet",
                    "--lines",
                    "37",
                    "--follow"
                ]
            );
            for manager in [
                Manager::Launchd {
                    domain: "system".into(),
                    target: "test".into(),
                },
                Manager::TaskScheduler {
                    name: "test".into(),
                },
                Manager::WindowsService,
            ] {
                plan.manager = manager;
                let Source::File(path) = source(&paths, &plan, options)? else {
                    crate::infra::error::bail!("file expected")
                };
                assert_eq!(path, paths.config.join("daemon.log"));
            }
            assert!(
                !paths.config.exists(),
                "viewing logs must not create state directories"
            );
        }
        Ok(())
    }

    #[test]
    fn missing_or_partial_installation_is_rejected_without_creating_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path().join("config"));
        let plan = crate::system::shared::services::artifacts::linux_plan(
            dir.path().join("daemon"),
            dir.path().join("service"),
        )?;
        let options = LogOptions {
            lines: 100,
            follow: false,
        };
        assert!(
            source(&paths, &plan, options)
                .err()
                .context("expected installation error")?
                .to_string()
                .contains("not installed")
        );
        fs::write(&plan.definition, &plan.content)?;
        assert!(
            source(&paths, &plan, options)
                .err()
                .context("expected installation error")?
                .to_string()
                .contains("incomplete")
        );
        assert!(!paths.config.exists());
        assert!(!plan.executable.exists());
        Ok(())
    }
}
