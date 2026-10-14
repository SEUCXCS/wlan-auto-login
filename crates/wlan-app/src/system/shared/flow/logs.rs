//! Run the complete log workflow without starting a daemon or mutating services.
use crate::infra::{
    error::{Message, Result, ensure},
    paths::Paths,
};
#[cfg(test)]
use crate::system::define::services::{LogSource as Source, Manager};
use crate::system::{
    define::services::{Artifacts, LogBackend, LogOptions, Plan},
    shared::services::artifacts::plan,
};

pub(crate) async fn run(
    paths: &Paths,
    options: LogOptions,
    output: &mut impl std::io::Write,
    stop: impl std::future::Future<Output = std::io::Result<()>>,
) -> Result<()> {
    run_with(
        paths,
        options,
        output,
        stop,
        &crate::system::shared::services::logs::FileBackend,
    )
    .await
}
pub(crate) async fn run_with(
    paths: &Paths,
    options: LogOptions,
    output: &mut impl std::io::Write,
    stop: impl std::future::Future<Output = std::io::Result<()>>,
    backend: &impl LogBackend,
) -> Result<()> {
    run_plan(paths, &plan(paths)?, options, output, stop, backend).await
}
async fn run_plan(
    paths: &Paths,
    plan: &Plan,
    options: LogOptions,
    output: &mut impl std::io::Write,
    stop: impl std::future::Future<Output = std::io::Result<()>>,
    backend: &impl LogBackend,
) -> Result<()> {
    // Inspect the selected installation before choosing any native log source.
    validate(plan)?;
    let source = backend.source(paths, plan, options)?;

    // Native readers receive the same cancellation/output contract as file logs.
    let result = backend.read(source, options, output, stop).await;

    // Closing a downstream pipe is a normal end to log consumption.
    match result {
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::BrokenPipe) =>
        {
            Ok(())
        }
        result => result,
    }
}
fn validate(plan: &Plan) -> Result<()> {
    let files = Artifacts::read(plan)?;
    ensure!(
        files.definition || files.executable,
        Message::ServiceNotInstalled
    );
    ensure!(files.complete(), Message::InstallationIncomplete);
    Ok(())
}

#[cfg(test)]
fn fixture_source(paths: &Paths, plan: &Plan, options: LogOptions) -> Result<Source> {
    validate(plan)?;
    // Reading diagnostics must also work when the service is stopped or its
    // registration is broken. No service manager query or mutation is needed.
    match &plan.manager {
        #[cfg(any(target_os = "linux", test))]
        Manager::Systemd { .. } => {
            crate::system::shared::services::logs::FileBackend.source(paths, plan, options)
        }
        #[cfg(any(target_os = "macos", test))]
        Manager::MacosAppService { .. } => {
            crate::system::macos::services::logs::source(paths, plan, options)
        }
        #[cfg(any(target_os = "windows", test))]
        Manager::TaskScheduler { .. } | Manager::WindowsService => {
            crate::system::windows::services::logs::source(paths, plan, options)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::error::Context;
    use crate::infra::scope::Scope;
    use crate::system::define::services::Manager;
    use std::fs;

    #[tokio::test]
    async fn common_log_flow_handles_follow_cancellation_and_closed_output_without_manager_commands()
    -> Result<()> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path().join("config"));
        fs::create_dir_all(&paths.config)?;
        let plan = crate::system::shared::services::artifacts::linux_plan(
            dir.path().join("daemon"),
            dir.path().join("definition"),
        )?;
        fs::write(&plan.definition, &plan.content)?;
        fs::write(&plan.executable, "mock, never executed")?;
        fs::write(paths.config.join("daemon.log"), "initial record\n")?;
        let mut output = Vec::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            run_plan(
                &paths,
                &plan,
                LogOptions {
                    lines: 1,
                    follow: true,
                },
                &mut output,
                async {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    Ok(())
                },
                &crate::system::shared::services::logs::FileBackend,
            ),
        )
        .await??;
        assert_eq!(String::from_utf8(output)?, "initial record\n");
        struct Closed;
        impl std::io::Write for Closed {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        run_plan(
            &paths,
            &plan,
            LogOptions {
                lines: 1,
                follow: false,
            },
            &mut Closed,
            std::future::pending(),
            &crate::system::shared::services::logs::FileBackend,
        )
        .await?;
        Ok(())
    }

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
            let Source::File(path) = fixture_source(&paths, &plan, options)?;
            assert_eq!(path, paths.config.join("daemon.log"));
            for manager in [
                Manager::MacosAppService {
                    domain: if scope == Scope::System {
                        "system".into()
                    } else {
                        "gui/501".into()
                    },
                    target: "test".into(),
                    executable: plan.executable.clone(),
                },
                Manager::TaskScheduler {
                    name: "test".into(),
                },
                Manager::WindowsService,
            ] {
                plan.manager = manager;
                let Source::File(path) = fixture_source(&paths, &plan, options)?;
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
            fixture_source(&paths, &plan, options)
                .err()
                .context("expected installation error")?
                .to_string()
                .contains("not installed")
        );
        fs::write(&plan.definition, &plan.content)?;
        assert!(
            fixture_source(&paths, &plan, options)
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
