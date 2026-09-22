#[cfg(any(target_os = "windows", test))]
use super::status::ServiceState;
use super::{
    Invocation, Plan, plan,
    status::{Artifacts, Manager, Runner, SystemRunner},
};
use crate::{config::Paths, control};
use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Start,
    Stop,
    Restart,
}

pub async fn manage(paths: &Paths, action: Action) -> Result<()> {
    if matches!(action, Action::Start | Action::Restart) {
        super::ensure_no_conflicting_install(paths)?;
    }
    manage_plan(paths, &plan(paths)?, action, &SystemRunner).await?;
    tracing::info!(
        "{}",
        match action {
            Action::Start => "已向系统服务管理器提交启动请求。",
            Action::Stop => "系统服务已停止。",
            Action::Restart => "已向系统服务管理器提交重启请求。",
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
        bail!("后台服务尚未安装，请先执行 install");
    }
    if !files.complete() {
        bail!("后台服务安装不完整，请重新执行 install");
    }
    let state = plan.manager.query(runner).await?;
    if !plan.manager.installed(&state) {
        bail!("后台服务未在系统中注册，请先执行 install");
    }
    // Do not send IPC Stop or use the daemon singleton lock: these commands
    // control only the registered service, never a manually launched process.
    match &plan.manager {
        #[cfg(any(target_os = "linux", test))]
        Manager::Systemd { system } => {
            let verb = match action {
                Action::Start => "start",
                Action::Stop => "stop",
                Action::Restart => "restart",
            };
            execute(
                runner,
                Invocation::new(
                    "systemctl",
                    [
                        if *system { "--system" } else { "--user" },
                        verb,
                        plan.manager.name(),
                    ],
                ),
            )
            .await
        }
        #[cfg(any(target_os = "macos", test))]
        Manager::Launchd { domain, target } => {
            use std::ffi::OsStr;
            match action {
                Action::Stop | Action::Restart if state.registered => {
                    execute(runner, Invocation::new("launchctl", ["bootout", target])).await?;
                }
                _ => {}
            }
            match action {
                Action::Start if state.registered => {
                    execute(runner, Invocation::new("launchctl", ["kickstart", target])).await
                }
                Action::Start | Action::Restart => {
                    execute(
                        runner,
                        Invocation::new(
                            "launchctl",
                            [
                                OsStr::new("bootstrap"),
                                OsStr::new(domain),
                                plan.definition.as_os_str(),
                            ],
                        ),
                    )
                    .await
                }
                Action::Stop => Ok(()),
            }
        }
        #[cfg(any(target_os = "windows", test))]
        Manager::TaskScheduler { name } => {
            let mut state = state;
            if matches!(action, Action::Stop | Action::Restart) && (state.running || state.pending)
            {
                execute(runner, Invocation::new("schtasks", ["/End", "/TN", name])).await?;
                state = wait_task_stopped(&plan.manager, runner).await?;
            }
            if matches!(action, Action::Start | Action::Restart) && !state.running && !state.pending
            {
                if !state.registered {
                    bail!("系统服务注册已被移除，请重新执行 install");
                }
                execute(runner, Invocation::new("schtasks", ["/Run", "/TN", name])).await?;
            }
            Ok(())
        }
        #[cfg(any(windows, test))]
        Manager::WindowsService => {
            let mut state = state;
            if matches!(action, Action::Stop | Action::Restart) && (state.running || state.pending)
            {
                execute(
                    runner,
                    Invocation::new("sc.exe", ["stop", "cxxy-wlanlogin"]),
                )
                .await?;
                state = wait_task_stopped(&plan.manager, runner).await?;
            }
            if matches!(action, Action::Start | Action::Restart) && !state.running && !state.pending
            {
                anyhow::ensure!(state.registered, "系统服务注册已被移除，请重新执行 install");
                execute(
                    runner,
                    Invocation::new("sc.exe", ["start", "cxxy-wlanlogin"]),
                )
                .await?;
            }
            Ok(())
        }
    }
}

async fn execute(runner: &impl Runner, command: Invocation) -> Result<()> {
    runner.output(&command).await?.check(&command)
}

#[cfg(any(target_os = "windows", test))]
pub(super) async fn wait_task_stopped(
    manager: &Manager,
    runner: &impl Runner,
) -> Result<ServiceState> {
    use anyhow::Context;
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
    .context("等待系统服务停止超时；未提交新的启动请求")?
}

#[cfg(test)]
mod tests {
    use super::super::status::tests::{FakeRunner, RUNNING, STOPPED};
    use super::*;
    use std::fs;

    fn fixture() -> Result<(tempfile::TempDir, Paths, Plan)> {
        let dir = tempfile::tempdir()?;
        let paths = Paths::test(dir.path().join("config"));
        let plan = super::super::linux_plan(dir.path().join("daemon"), dir.path().join("service"))?;
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
            paths.scope = crate::scope::Scope::System;
            let plan = super::super::linux_system_plan(
                dir.path().join("daemon"),
                dir.path().join("service"),
            )?;
            installed(&plan)?;
            let runner = FakeRunner::new([(true, STOPPED), (true, "")]);
            manage_plan(&paths, &plan, action, &runner).await?;
            assert_eq!(
                calls(&runner)[1],
                ["systemctl", "--system", verb, "cxxy-wlanlogin.service"]
            );

            let plan = super::super::windows_system_plan(
                dir.path().join("daemon.exe"),
                dir.path().join("service.json"),
            )?;
            installed(&plan)?;
            let runner = FakeRunner::new([(true, "Installed=1\nState=1\nEnabled=1"), (true, "")]);
            manage_plan(&paths, &plan, action, &runner).await?;
            if matches!(action, Action::Stop) {
                assert_eq!(calls(&runner).len(), 1);
            } else {
                assert_eq!(calls(&runner)[1], ["sc.exe", "start", "cxxy-wlanlogin"]);
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn scm_restart_waits_for_stop_and_aborts_on_stop_failure() -> Result<()> {
        let (dir, mut paths, _) = fixture()?;
        paths.scope = crate::scope::Scope::System;
        let plan = super::super::windows_system_plan(
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
        assert_eq!(submitted[1], ["sc.exe", "stop", "cxxy-wlanlogin"]);
        assert_eq!(submitted[2][0], "powershell.exe");
        assert_eq!(submitted[3][0], "powershell.exe");
        assert_eq!(submitted[4], ["sc.exe", "start", "cxxy-wlanlogin"]);
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
                .ok_or_else(|| anyhow::anyhow!("missing installation accepted"))?;
            assert!(error.to_string().contains("尚未安装"));
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
                ["systemctl", "--user", verb, "cxxy-wlanlogin.service"]
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

    const MAC_RUNNING: &str =
        "gui/501/org.seucxcs.cxxy-wlanlogin = {\nstate = running\npid = 123\n}";
    const MAC_DOMAIN: &str = "gui/501 = {\nservices = {\n}\n}";

    #[tokio::test]
    async fn macos_stop_and_restart_keep_the_installed_plist() -> Result<()> {
        for action in [Action::Stop, Action::Restart] {
            let (dir, paths, _) = fixture()?;
            let plan = super::super::macos_plan(
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
                ["launchctl", "bootout", "gui/501/org.seucxcs.cxxy-wlanlogin"]
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
            let plan = super::super::macos_plan(
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
        let plan = super::super::windows_plan(
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
            ["schtasks", "/End", "/TN", "cxxy-wlanlogin-S-1-5-21-123"]
        );
        assert_eq!(calls[2][0], "powershell.exe");
        assert_eq!(calls[3][0], "powershell.exe");
        assert_eq!(
            calls[4],
            ["schtasks", "/Run", "/TN", "cxxy-wlanlogin-S-1-5-21-123"]
        );
        assert_eq!(fs::read_to_string(&plan.definition)?, plan.content);
        Ok(())
    }

    #[tokio::test]
    async fn windows_stopped_task_can_start_or_restart_and_stop_is_idempotent() -> Result<()> {
        for action in [Action::Start, Action::Restart, Action::Stop] {
            let (dir, paths, _) = fixture()?;
            let plan = super::super::windows_plan(
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
                    ["schtasks", "/Run", "/TN", "cxxy-wlanlogin-S-1-5-21-123"]
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn windows_failed_stop_never_submits_restart() -> Result<()> {
        let (dir, paths, _) = fixture()?;
        let plan = super::super::windows_plan(
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
