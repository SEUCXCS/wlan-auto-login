use super::*;
use crate::scope::Scope;
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
