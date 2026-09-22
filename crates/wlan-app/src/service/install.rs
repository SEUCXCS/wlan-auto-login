//! Install or remove service artifacts, restoring previous files on failure.
use super::definition::{Plan, assert_owned, ensure_no_conflicting_install, plan};
#[cfg(windows)]
use super::{command::SystemRunner, lifecycle};
use crate::{
    config::{Paths, private_dir},
    control,
};
use anyhow::{Context, Result, bail};
use std::{fs, path::Path, time::Duration};

pub async fn install(paths: &Paths) -> Result<()> {
    ensure_no_conflicting_install(paths)?;
    let plan = plan(paths)?;
    #[cfg(target_os = "linux")]
    let plan = {
        let mut plan = plan;
        if paths.scope == crate::scope::Scope::System {
            plan.content = plan.content.replace(
                "# CREDENTIALS\n",
                &crate::credentials::linux_credential_directive(paths)?,
            );
        }
        plan
    };
    install_plan(paths, plan, &std::env::current_exe()?).await
}

async fn install_plan(paths: &Paths, plan: Plan, source: &Path) -> Result<()> {
    anyhow::ensure!(plan.scope == paths.scope, "服务计划与安装范围不一致");
    let _service_lock = control::service_lock(paths)?;
    assert_owned(&plan.definition)?;
    let source = source.canonicalize()?;
    let binary_dir = plan.executable.parent().context("安装路径无父目录")?;
    private_dir(binary_dir)?;
    #[cfg(windows)]
    if paths.scope == crate::scope::Scope::System {
        crate::config::restrict_windows(binary_dir, true)?;
    }
    #[cfg(unix)]
    if paths.scope == crate::scope::Scope::System {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(binary_dir, fs::Permissions::from_mode(0o755))?;
    }
    if let Some(parent) = plan.definition.parent() {
        fs::create_dir_all(parent)?;
        #[cfg(windows)]
        if paths.scope == crate::scope::Scope::System {
            crate::config::restrict_windows(parent, true)?;
        }
    }
    let old_definition = Backup::capture(&plan.definition)?;
    let old_executable = Backup::capture(&plan.executable)?;
    // Stage the executable before changing a running service.
    let staged = tempfile::NamedTempFile::new_in(binary_dir)?;
    fs::copy(&source, staged.path())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            staged.path(),
            fs::Permissions::from_mode(if paths.scope == crate::scope::Scope::System {
                0o755
            } else {
                0o700
            }),
        )?;
    }
    stop_before_replacing(paths, &plan).await?;
    let outcome: Result<()> = async {
        if source != plan.executable.canonicalize().unwrap_or_default() {
            staged.persist(&plan.executable).map_err(|e| e.error)?;
        }
        #[cfg(target_os = "macos")]
        if paths.scope == crate::scope::Scope::System {
            crate::credentials::authorize_binary(&plan.executable)?;
        }
        write_definition(&plan.definition, plan.content.as_bytes())?;
        for command in &plan.start {
            command.run().await?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = outcome {
        // Attempt every rollback step, and report incomplete recovery accurately.
        let mut rollback_errors = Vec::new();
        if let Err(e) = stop_before_replacing(paths, &plan).await {
            rollback_errors.push(e.to_string());
        }
        for command in &plan.remove {
            if let Err(e) = command.run().await {
                rollback_errors.push(e.to_string());
            }
        }
        for result in [
            old_executable.restore(&plan.executable),
            old_definition.restore(&plan.definition),
        ] {
            if let Err(e) = result {
                rollback_errors.push(e.to_string());
            }
        }
        #[cfg(target_os = "macos")]
        if paths.scope == crate::scope::Scope::System
            && old_executable.existed()
            && let Err(e) = crate::credentials::authorize_binary(&plan.executable)
        {
            rollback_errors.push(e.to_string());
        }
        let commands = if old_definition.existed() {
            &plan.start
        } else {
            &plan.reload
        };
        for command in commands {
            if let Err(e) = command.run().await {
                rollback_errors.push(e.to_string());
            }
        }
        if !rollback_errors.is_empty() {
            return Err(error).context(format!(
                "安装失败，回滚仍有错误：{}",
                rollback_errors.join("；")
            ));
        }
        return Err(error).context("安装失败，已回滚安装文件");
    }
    tracing::info!(
        "{}服务已安装并启动，{}。",
        paths.scope.label(),
        if paths.scope == crate::scope::Scope::System {
            "今后随开机自动运行"
        } else {
            "今后用户登录系统后自动运行"
        }
    );
    Ok(())
}

pub async fn uninstall(paths: &Paths) -> Result<()> {
    let _service_lock = control::service_lock(paths)?;
    let plan = plan(paths)?;
    assert_owned(&plan.definition)?;
    if plan.definition.exists() {
        stop_before_replacing(paths, &plan).await?;
        for command in &plan.remove {
            command.run().await?;
        }
    }
    control::stop(paths).await;
    wait_stopped(paths).await?;
    remove_if_present(&plan.definition)?;
    for command in &plan.reload {
        command.run().await?;
    }
    remove_if_present(&plan.executable).context("服务已卸载，但安装副本仍在使用，请关闭后删除")?;
    tracing::info!("后台服务已卸载；已保存的账号和密码仍保留。");
    Ok(())
}

async fn stop_before_replacing(paths: &Paths, plan: &Plan) -> Result<()> {
    for command in &plan.stop {
        command.run().await?;
    }
    control::stop(paths).await;
    wait_stopped(paths).await?;
    #[cfg(windows)]
    lifecycle::wait_task_stopped(&plan.manager, &SystemRunner).await?;
    Ok(())
}

async fn wait_stopped(paths: &Paths) -> Result<()> {
    for _ in 0..250 {
        if control::singleton(paths).is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("daemon 尚未退出，安装文件没有被替换；请停止正在运行的 daemon 后重试")
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
// Service directories may already contain other applications' files; do not chmod them.
fn write_definition(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new_in(path.parent().context("服务文件无父目录")?)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o644))?;
    }
    Ok(())
}
struct Backup(Option<tempfile::NamedTempFile>);
impl Backup {
    fn capture(path: &Path) -> Result<Self> {
        match fs::metadata(path) {
            Ok(_) => {
                let backup =
                    tempfile::NamedTempFile::new_in(path.parent().context("备份路径无父目录")?)?;
                fs::copy(path, backup.path())?;
                Ok(Self(Some(backup)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self(None)),
            Err(e) => Err(e.into()),
        }
    }
    fn existed(&self) -> bool {
        self.0.is_some()
    }
    fn restore(&self, path: &Path) -> Result<()> {
        match &self.0 {
            Some(backup) => {
                fs::copy(backup.path(), path)?;
                Ok(())
            }
            None => remove_if_present(path),
        }
    }
}

#[cfg(test)]
mod tests;
