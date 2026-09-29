#[cfg(any(target_os = "windows", test))]
use super::Manager;
#[cfg(any(target_os = "windows", test))]
use super::ServiceState;
use super::artifacts::{Artifacts, Plan, plan};
use crate::infra::command::{Invocation, Runner, SystemRunner};
use crate::infra::error::Message;
use crate::infra::error::{Result, bail};
pub(crate) use crate::system::define::services::Action;
use crate::system::define::services::ServiceManager;
use crate::{infra::control, infra::paths::Paths};

pub async fn manage(paths: &Paths, action: Action) -> Result<()> {
    if matches!(action, Action::Start | Action::Restart) {
        super::ensure_no_conflicting_install(paths)?;
    }
    manage_plan(paths, &plan(paths)?, action, &SystemRunner).await?;
    tracing::info!(
        "{}",
        match action {
            Action::Start => "Service start requested",
            Action::Stop => "Service stopped",
            Action::Restart => "Service restart requested",
        }
    );
    Ok(())
}

async fn manage_plan(
    paths: &Paths,
    plan: &Plan,
    action: Action,
    runner: &impl Runner,
) -> Result<()> {
    let _service_lock = control::service_lock(paths)?;
    let files = Artifacts::read(plan)?;
    if !files.definition && !files.executable {
        bail!(Message::ServiceNotInstalled);
    }
    if !files.complete() {
        bail!(Message::InstallationIncomplete);
    }
    let state = plan.manager.query(runner).await?;
    if !plan.manager.installed(&state) {
        bail!(Message::ServiceNotRegistered);
    }
    // Do not send IPC Stop or use the daemon singleton lock: these commands
    // control only the registered service, never a manually launched process.
    plan.manager.manage(plan, action, state, runner).await
}

pub(crate) async fn execute(runner: &impl Runner, command: Invocation) -> Result<()> {
    runner.output(&command).await?.check(&command)
}

#[cfg(any(target_os = "windows", test))]
pub(crate) async fn wait_task_stopped(
    manager: &Manager,
    runner: &impl Runner,
) -> Result<ServiceState> {
    use crate::infra::error::Context;
    use std::time::Duration;
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let state = manager.query(runner).await?;
            if !state.running && !state.pending {
                return Ok(state);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .context(Message::TimedOutWaitingForServiceStopNoNew)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::command::tests::{FakeRunner, RUNNING, STOPPED};
    use std::fs;

    fn fixture() -> Result<(tempfile::TempDir, Paths, Plan)> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path().join("config"));
        let plan = crate::system::shared::services::artifacts::linux_plan(
            dir.path().join("daemon"),
            dir.path().join("service"),
        )?;
        Ok((dir, paths, plan))
    }

    fn installed(plan: &Plan) -> Result<()> {
        fs::write(&plan.definition, &plan.content)?;
        fs::write(&plan.executable, "mock executable, never run")?;
        Ok(())
    }

    fn calls(runner: &FakeRunner) -> Vec<Vec<String>> {
        runner
            .calls
            .borrow()
            .iter()
            .map(|call| {
                std::iter::once(&call.program)
                    .chain(call.args.iter())
                    .map(|argument| argument.to_string_lossy().into_owned())
                    .collect()
            })
            .collect()
    }

    #[tokio::test]
    async fn system_controls_use_systemd_and_scm_in_the_selected_scope() -> Result<()> {
        for (action, verb) in [
            (Action::Start, "start"),
            (Action::Stop, "stop"),
            (Action::Restart, "restart"),
        ] {
            let (dir, mut paths, _) = fixture()?;
            paths.scope = crate::infra::scope::Scope::System;
            let plan = crate::system::shared::services::artifacts::linux_system_plan(
                dir.path().join("daemon"),
                dir.path().join("service"),
            )?;
            installed(&plan)?;
            let runner = FakeRunner::new([(true, STOPPED), (true, "")]);
            manage_plan(&paths, &plan, action, &runner).await?;
            assert_eq!(
                calls(&runner)[1],
                [
                    "systemctl",
                    "--system",
                    verb,
                    "dev.cxcs.tools.wlanlogin.service"
                ]
            );

            let plan = crate::system::shared::services::artifacts::windows_system_plan(
                dir.path().join("daemon.exe"),
                dir.path().join("service.json"),
            )?;
            installed(&plan)?;
            let runner = FakeRunner::new([(true, "Installed=1\nState=1\nEnabled=1"), (true, "")]);
            manage_plan(&paths, &plan, action, &runner).await?;
            if matches!(action, Action::Stop) {
                assert_eq!(calls(&runner).len(), 1);
            } else {
                assert_eq!(
                    calls(&runner)[1],
                    ["sc.exe", "start", "dev.cxcs.tools.wlanlogin"]
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn scm_restart_waits_for_stop_and_aborts_on_stop_failure() -> Result<()> {
        let (dir, mut paths, _) = fixture()?;
        paths.scope = crate::infra::scope::Scope::System;
        let plan = crate::system::shared::services::artifacts::windows_system_plan(
            dir.path().join("daemon.exe"),
            dir.path().join("service.json"),
        )?;
        installed(&plan)?;
        let runner = FakeRunner::new([
            (true, "Installed=1\nState=4"),
            (true, ""),
            (true, "Installed=1\nState=3"),
            (true, "Installed=1\nState=1"),
            (true, ""),
        ]);
        manage_plan(&paths, &plan, Action::Restart, &runner).await?;
        let submitted = calls(&runner);
        assert_eq!(submitted[1], ["sc.exe", "stop", "dev.cxcs.tools.wlanlogin"]);
        assert_eq!(submitted[2][0], "powershell.exe");
        assert_eq!(submitted[3][0], "powershell.exe");
        assert_eq!(
            submitted[4],
            ["sc.exe", "start", "dev.cxcs.tools.wlanlogin"]
        );
        let runner = FakeRunner::new([(true, "Installed=1\nState=4"), (false, "stop failed")]);
        assert!(
            manage_plan(&paths, &plan, Action::Restart, &runner)
                .await
                .is_err()
        );
        assert_eq!(calls(&runner).len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn all_controls_reject_missing_and_partial_installations_without_commands() -> Result<()>
    {
        for action in [Action::Start, Action::Stop, Action::Restart] {
            let (_dir, paths, plan) = fixture()?;
            let runner = FakeRunner::default();
            let error = manage_plan(&paths, &plan, action, &runner)
                .await
                .err()
                .ok_or_else(|| crate::infra::error::error!("missing installation accepted"))?;
            assert!(error.to_string().contains("not installed"));
            assert!(runner.calls.borrow().is_empty());
            fs::write(&plan.definition, &plan.content)?;
            assert!(manage_plan(&paths, &plan, action, &runner).await.is_err());
            assert!(runner.calls.borrow().is_empty());
            assert!(!plan.executable.exists());
        }
        Ok(())
    }

    #[tokio::test]
    async fn missing_registration_and_failed_queries_never_issue_control_commands() -> Result<()> {
        for action in [Action::Start, Action::Stop, Action::Restart] {
            for (success, reply) in [(false, "LoadState=not-found"), (false, "bus unavailable")] {
                let (_dir, paths, plan) = fixture()?;
                installed(&plan)?;
                let runner = FakeRunner::new([(success, reply)]);
                assert!(manage_plan(&paths, &plan, action, &runner).await.is_err());
                assert_eq!(runner.calls.borrow().len(), 1);
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn linux_controls_only_the_service_and_preserves_manual_daemon_state() -> Result<()> {
        for (action, verb) in [
            (Action::Start, "start"),
            (Action::Stop, "stop"),
            (Action::Restart, "restart"),
        ] {
            let (_dir, paths, plan) = fixture()?;
            installed(&plan)?;
            // A manual daemon remains exclusively locked and its pause state is intact.
            let _manual_daemon = control::singleton(&paths)?;
            control::State {
                paused: true,
                session: Some("manual-session".into()),
                revision: 9,
            }
            .save(&paths)?;
            let runner = FakeRunner::new([(true, STOPPED), (true, "")]);
            manage_plan(&paths, &plan, action, &runner).await?;
            assert_eq!(
                calls(&runner)[1],
                [
                    "systemctl",
                    "--user",
                    verb,
                    "dev.cxcs.tools.wlanlogin.service"
                ]
            );
            assert!(control::singleton(&paths).is_err());
            assert!(control::State::load(&paths)?.paused);
            assert_eq!(control::State::load(&paths)?.revision, 9);
            assert_eq!(fs::read_to_string(&plan.definition)?, plan.content);
            assert_eq!(
                fs::read_to_string(&plan.executable)?,
                "mock executable, never run"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn command_failure_is_not_reported_as_success() -> Result<()> {
        let (_dir, paths, plan) = fixture()?;
        installed(&plan)?;
        let runner = FakeRunner::new([(true, RUNNING), (false, "access denied")]);
        assert!(
            manage_plan(&paths, &plan, Action::Stop, &runner)
                .await
                .is_err()
        );
        assert_eq!(runner.calls.borrow().len(), 2);
        Ok(())
    }

    const MAC_RUNNING: &str = "gui/501/dev.cxcs.tools.wlanlogin = {\nstate = running\npid = 123\n}";
    const MAC_DOMAIN: &str = "gui/501 = {\nservices = {\n}\n}";

    #[tokio::test]
    async fn macos_stop_and_restart_keep_the_installed_plist() -> Result<()> {
        for action in [Action::Stop, Action::Restart] {
            let (dir, paths, _) = fixture()?;
            let plan = crate::system::shared::services::artifacts::macos_plan(
                dir.path().join("daemon"),
                dir.path().join("service.plist"),
                dir.path().join("log"),
                501,
            )?;
            installed(&plan)?;
            let runner = FakeRunner::new([(true, MAC_RUNNING), (true, ""), (true, "")]);
            manage_plan(&paths, &plan, action, &runner).await?;
            let calls = calls(&runner);
            assert_eq!(
                calls[1],
                ["launchctl", "bootout", "gui/501/dev.cxcs.tools.wlanlogin"]
            );
            if matches!(action, Action::Restart) {
                assert_eq!(
                    calls[2],
                    [
                        "launchctl",
                        "bootstrap",
                        "gui/501",
                        plan.definition.to_str().unwrap_or_default()
                    ]
                );
            } else {
                assert_eq!(calls.len(), 2);
            }
            assert_eq!(fs::read_to_string(&plan.definition)?, plan.content);
        }
        Ok(())
    }

    #[tokio::test]
    async fn macos_start_or_restart_after_stop_bootstraps_the_existing_plist() -> Result<()> {
        for action in [Action::Start, Action::Restart, Action::Stop] {
            let (dir, paths, _) = fixture()?;
            let plan = crate::system::shared::services::artifacts::macos_plan(
                dir.path().join("daemon"),
                dir.path().join("service.plist"),
                dir.path().join("log"),
                501,
            )?;
            installed(&plan)?;
            let runner = FakeRunner::new([(false, "job absent"), (true, MAC_DOMAIN), (true, "")]);
            manage_plan(&paths, &plan, action, &runner).await?;
            let calls = calls(&runner);
            if matches!(action, Action::Stop) {
                assert_eq!(calls.len(), 2);
            } else {
                assert_eq!(
                    calls[2],
                    [
                        "launchctl",
                        "bootstrap",
                        "gui/501",
                        plan.definition.to_str().unwrap_or_default()
                    ]
                );
            }
        }
        Ok(())
    }

    const WIN_RUNNING: &str = "Installed=1\nState=4\nEnabled=1";
    const WIN_QUEUED: &str = "Installed=1\nState=2\nEnabled=1";
    const WIN_STOPPED: &str = "Installed=1\nState=3\nEnabled=1";

    #[tokio::test]
    async fn windows_restart_waits_until_old_task_exits_before_run() -> Result<()> {
        let (dir, paths, _) = fixture()?;
        let plan = crate::system::shared::services::artifacts::windows_plan(
            dir.path().join("daemon.exe"),
            dir.path().join("task.xml"),
            "S-1-5-21-123",
        )?;
        installed(&plan)?;
        let runner = FakeRunner::new([
            (true, WIN_RUNNING),
            (true, ""),
            (true, WIN_QUEUED),
            (true, WIN_STOPPED),
            (true, ""),
        ]);
        manage_plan(&paths, &plan, Action::Restart, &runner).await?;
        let calls = calls(&runner);
        assert_eq!(
            calls[1],
            [
                "schtasks",
                "/End",
                "/TN",
                "dev.cxcs.tools.wlanlogin-S-1-5-21-123"
            ]
        );
        assert_eq!(calls[2][0], "powershell.exe");
        assert_eq!(calls[3][0], "powershell.exe");
        assert_eq!(
            calls[4],
            [
                "schtasks",
                "/Run",
                "/TN",
                "dev.cxcs.tools.wlanlogin-S-1-5-21-123"
            ]
        );
        assert_eq!(fs::read_to_string(&plan.definition)?, plan.content);
        Ok(())
    }

    #[tokio::test]
    async fn windows_stopped_task_can_start_or_restart_and_stop_is_idempotent() -> Result<()> {
        for action in [Action::Start, Action::Restart, Action::Stop] {
            let (dir, paths, _) = fixture()?;
            let plan = crate::system::shared::services::artifacts::windows_plan(
                dir.path().join("daemon.exe"),
                dir.path().join("task.xml"),
                "S-1-5-21-123",
            )?;
            installed(&plan)?;
            let runner = FakeRunner::new([(true, WIN_STOPPED), (true, "")]);
            manage_plan(&paths, &plan, action, &runner).await?;
            let calls = calls(&runner);
            if matches!(action, Action::Stop) {
                assert_eq!(calls.len(), 1);
            } else {
                assert_eq!(
                    calls[1],
                    [
                        "schtasks",
                        "/Run",
                        "/TN",
                        "dev.cxcs.tools.wlanlogin-S-1-5-21-123"
                    ]
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn windows_failed_stop_never_submits_restart() -> Result<()> {
        let (dir, paths, _) = fixture()?;
        let plan = crate::system::shared::services::artifacts::windows_plan(
            dir.path().join("daemon.exe"),
            dir.path().join("task.xml"),
            "S-1-5-21-123",
        )?;
        installed(&plan)?;
        let runner = FakeRunner::new([(true, WIN_RUNNING), (false, "cannot stop")]);
        assert!(
            manage_plan(&paths, &plan, Action::Restart, &runner)
                .await
                .is_err()
        );
        assert_eq!(runner.calls.borrow().len(), 2);
        Ok(())
    }
}
