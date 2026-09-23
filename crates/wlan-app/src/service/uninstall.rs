//! Unregister services and optionally remove all data owned by the selected scope.
use super::{
    command::{Invocation, Runner, SystemRunner},
    definition::{Plan, assert_owned, plan},
    manager::Manager,
};
use crate::{cleanup, config::Paths, control, credentials};
use anyhow::{Context, Result, ensure};
use std::{future::Future, time::Duration};

pub async fn uninstall(paths: &Paths, purge: bool) -> Result<()> {
    uninstall_plan(
        paths,
        &plan(paths)?,
        purge,
        &SystemRunner,
        credentials::delete(paths),
    )
    .await?;
    if purge {
        tracing::info!(
            "{}服务及其应用日志、配置、运行文件和凭据已清理。",
            paths.scope.label()
        );
        tracing::info!("操作系统共享日志和服务审计记录由系统管理，不属于本程序可单独删除的文件。");
    } else {
        tracing::info!("后台服务已卸载；已保存的账号和密码仍保留。");
    }
    Ok(())
}

async fn uninstall_plan(
    paths: &Paths,
    plan: &Plan,
    purge: bool,
    runner: &impl Runner,
    delete_credentials: impl Future<Output = Result<()>>,
) -> Result<()> {
    ensure!(plan.scope == paths.scope, "服务计划与安装范围不一致");
    for directory in [&paths.config, &paths.runtime, &paths.secrets] {
        cleanup::validate_directory(directory)?;
    }
    assert_owned(&plan.definition)?;
    let service_lock = control::service_lock(paths)?;
    // Query registrations even when the definition was manually removed.
    let state = plan.manager.query(runner).await?;
    if state.running || state.pending {
        for command in &plan.stop {
            execute(runner, command).await?;
        }
        wait_for_manager(&plan.manager, runner, false).await?;
    }
    control::stop(paths).await;
    control::wait_stopped(paths).await?;
    let daemon_lock = control::singleton(paths)?;
    let operation_lock = control::operation_lock(paths).await?;
    if state.registered {
        for command in &plan.remove {
            execute(runner, command).await?;
        }
    }
    remove_enable_link(plan)?;
    cleanup::remove_file(&plan.definition)?;
    for command in &plan.reload {
        execute(runner, command).await?;
    }
    wait_for_manager(&plan.manager, runner, true).await?;

    if purge {
        delete_credentials
            .await
            .context("服务已卸载，但凭据删除未完成；可再次执行 uninstall --purge")?;
    }
    cleanup::remove_file(&plan.executable)
        .context("服务已卸载，但安装副本仍在使用或无法删除；请关闭该副本后重试")?;
    if purge {
        for name in ["preferences.json", "control.json", "daemon.log"] {
            cleanup::remove_file(&paths.config.join(name))?;
        }
        for name in ["daemon-endpoint.json", "daemon.sock"] {
            cleanup::remove_file(&paths.runtime.join(name))?;
        }
        // Windows requires the handles closed before their lock files are removed.
        drop(operation_lock);
        drop(daemon_lock);
        for name in ["operation.lock", "daemon.lock"] {
            cleanup::remove_file(&paths.runtime.join(name))?;
        }
        drop(service_lock);
        cleanup::remove_file(&paths.config.join("service.lock"))?;
        for directory in [&paths.runtime, &paths.secrets, &paths.config] {
            cleanup::prune_empty_directories(directory)?;
        }
        cleanup::prune_empty_directories(plan.executable.parent().context("安装路径没有父目录")?)?;
    }
    Ok(())
}

fn remove_enable_link(plan: &Plan) -> Result<()> {
    #[cfg(any(target_os = "linux", test))]
    match &plan.manager {
        Manager::Systemd { system } => {
            let parent = plan.definition.parent().context("服务文件没有父目录")?;
            let name = plan.definition.file_name().context("服务文件没有名称")?;
            let path = parent
                .join(if *system {
                    "multi-user.target.wants"
                } else {
                    "default.target.wants"
                })
                .join(name);
            cleanup::validate_directory(path.parent().context("服务链接没有父目录")?)?;
            match std::fs::read_link(&path) {
                Ok(target) => {
                    ensure!(
                        target == plan.definition
                            || target == std::path::Path::new("..").join(name),
                        "拒绝删除指向其他服务的自动启动链接：{}",
                        path.display()
                    );
                    cleanup::remove_file(&path)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error).context("无法清理服务自动启动链接"),
            }
        }
        #[cfg(test)]
        _ => {}
    }
    #[cfg(not(any(target_os = "linux", test)))]
    let _ = plan;
    Ok(())
}

async fn execute(runner: &impl Runner, command: &Invocation) -> Result<()> {
    let output = runner.output(command).await?;
    if !command.optional {
        output.check(command)?;
    }
    Ok(())
}

async fn wait_for_manager(manager: &Manager, runner: &impl Runner, removed: bool) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(25), async {
        loop {
            let state = manager.query(runner).await?;
            if !state.running && !state.pending && (!removed || !state.registered) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .context(if removed {
        "等待服务注销超时，尚未清理应用数据"
    } else {
        "等待服务停止超时，尚未删除安装文件"
    })?
}

#[cfg(test)]
mod tests;
