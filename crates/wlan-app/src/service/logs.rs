#[cfg(target_os = "linux")]
use super::command::SystemRunner;
#[cfg(any(target_os = "linux", test))]
use super::command::{Invocation, Runner};
use super::{
    definition::{Artifacts, Plan, plan},
    manager::Manager,
};
use crate::infra::paths::Paths;
#[cfg(any(target_os = "linux", test))]
use anyhow::Context;
use anyhow::{Result, ensure};
#[cfg(any(target_os = "linux", test))]
use std::io::Write;
use std::path::PathBuf;

pub(crate) use crate::infra::file_log::LogOptions;
use crate::infra::file_log::file_logs;

#[derive(Debug)]
enum Source {
    #[cfg(any(target_os = "linux", test))]
    Journal(Invocation),
    File(PathBuf),
}

fn source(paths: &Paths, plan: &Plan, options: LogOptions) -> Result<Source> {
    let files = Artifacts::read(plan)?;
    ensure!(
        files.definition || files.executable,
        "后台服务尚未安装，请先执行 install"
    );
    ensure!(files.complete(), "后台服务安装不完整，请重新执行 install");
    // Reading diagnostics must also work when the service is stopped or its
    // registration is broken. No service manager query or mutation is needed.
    match &plan.manager {
        #[cfg(any(target_os = "linux", test))]
        Manager::Systemd { system } => {
            // Inspect the installed definition, not the newly generated plan:
            // older installations still send their application output to journal.
            if std::fs::read_to_string(&plan.definition)?
                .lines()
                .any(|line| line.trim() == "# ApplicationLog=daemon.log")
            {
                return Ok(Source::File(paths.config.join("daemon.log")));
            }
            let mut command = Invocation::new(
                "journalctl",
                [
                    if *system { "--system" } else { "--user" },
                    "--unit",
                    plan.manager.name(),
                    "--no-pager",
                    "--output=cat",
                    "--quiet",
                    "--lines",
                    &options.lines.to_string(),
                ],
            );
            if options.follow {
                command.args.push("--follow".into());
            }
            Ok(Source::Journal(command))
        }
        #[cfg(any(target_os = "macos", test))]
        Manager::Launchd { .. } => {
            let _ = options;
            Ok(Source::File(paths.config.join("daemon.log")))
        }
        #[cfg(any(windows, test))]
        Manager::TaskScheduler { .. } | Manager::WindowsService => {
            let _ = options;
            Ok(Source::File(paths.config.join("daemon.log")))
        }
    }
}

pub async fn logs(paths: &Paths, options: LogOptions) -> Result<()> {
    let source = source(paths, &plan(paths)?, options)?;
    let result = match source {
        #[cfg(target_os = "linux")]
        Source::Journal(command) if options.follow => {
            follow_journal(&command, tokio::signal::ctrl_c()).await
        }
        #[cfg(target_os = "linux")]
        Source::Journal(command) => {
            journal_snapshot(&command, &SystemRunner, &mut std::io::stdout()).await
        }
        Source::File(path) => file_logs(&path, options).await,
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

#[cfg(any(target_os = "linux", test))]
async fn journal_snapshot(
    command: &Invocation,
    runner: &impl Runner,
    output: &mut impl Write,
) -> Result<()> {
    let result = runner.output(command).await?;
    result.check(command)?;
    if result.stdout.is_empty() {
        tracing::info!("暂无可读取的服务日志");
    } else {
        output.write_all(result.stdout.as_bytes())?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn follow_journal(
    command: &Invocation,
    stop: impl std::future::Future<Output = std::io::Result<()>>,
) -> Result<()> {
    use std::process::Stdio;
    let mut child = tokio::process::Command::new(&command.program)
        .args(&command.args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .context("无法读取 journal 日志，请确认 journalctl 可用且当前用户有读取权限")?;
    tokio::select! {
        result = child.wait() => {
            let status = result?;
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                if matches!(status.signal(), Some(libc::SIGINT | libc::SIGPIPE)) { return Ok(()); }
            }
            ensure!(status.success() || status.code() == Some(130), "journalctl 执行失败：{status}");
            Ok(())
        },
        signal = stop => {
            signal.context("无法监听 Ctrl+C")?;
            child.start_kill()?;
            child.wait().await?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::scope::Scope;
    use crate::service::testing::FakeRunner;
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
            let mut plan = crate::service::definition::linux_plan(
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
                anyhow::bail!("new Linux service must use a removable application log")
            };
            assert_eq!(path, paths.config.join("daemon.log"));
            fs::write(
                &plan.definition,
                plan.content.replace("# ApplicationLog=daemon.log\n", ""),
            )?;
            let Source::Journal(command) = source(&paths, &plan, options)? else {
                anyhow::bail!("journal expected")
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
                    "cxxy-wlanlogin.service",
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
                    anyhow::bail!("file expected")
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
        let plan = crate::service::definition::linux_plan(
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
                .contains("尚未安装")
        );
        fs::write(&plan.definition, &plan.content)?;
        assert!(
            source(&paths, &plan, options)
                .err()
                .context("expected installation error")?
                .to_string()
                .contains("不完整")
        );
        assert!(!paths.config.exists());
        assert!(!plan.executable.exists());
        Ok(())
    }

    #[tokio::test]
    #[cfg(target_os = "linux")]
    async fn journal_follow_cancels_the_child_and_reports_failures() -> Result<()> {
        let command = Invocation::new("sleep", ["30"]);
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            follow_journal(&command, std::future::ready(Ok(()))),
        )
        .await??;
        let command = Invocation::new("false", std::iter::empty::<&str>());
        assert!(
            follow_journal(&command, std::future::pending())
                .await
                .is_err()
        );
        Ok(())
    }

    #[tokio::test]
    async fn journal_keeps_original_messages_and_reports_access_failures() -> Result<()> {
        let command = Invocation::new("journalctl", ["--user"]);
        let runner = FakeRunner::new([(true, "[INFO 2026-09-22 10:30:00] 日志\n")]);
        let mut output = Vec::new();
        journal_snapshot(&command, &runner, &mut output).await?;
        assert_eq!(output, "[INFO 2026-09-22 10:30:00] 日志\n".as_bytes());
        let runner = FakeRunner::new([(false, "access denied")]);
        assert!(
            journal_snapshot(&command, &runner, &mut Vec::new())
                .await
                .is_err()
        );
        let runner = FakeRunner::new([(true, "")]);
        let mut output = Vec::new();
        journal_snapshot(&command, &runner, &mut output).await?;
        assert!(output.is_empty());
        Ok(())
    }
}
