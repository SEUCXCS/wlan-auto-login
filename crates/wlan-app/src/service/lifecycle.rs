#[cfg(any(target_os = "windows", test))]
use super::manager::ServiceState;
use super::{
    command::{Invocation, Runner, SystemRunner},
    definition::{Artifacts, Plan, plan},
    manager::Manager,
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
mod tests;
