//! Report installation, running state and installed binary version.
use super::{
    command::{Invocation, Runner, SystemRunner},
    definition::{Artifacts, Plan, plan},
};
use crate::infra::paths::Paths;
use anyhow::{Context, Result};

pub async fn status(paths: &Paths, discover: bool) -> Result<()> {
    let mut plans = vec![plan(paths)?];
    if discover && paths.scope == crate::infra::scope::Scope::User {
        plans.push(plan(&Paths::new(crate::infra::scope::Scope::System)?)?);
    }
    let mut reports = Vec::new();
    let mut errors = Vec::new();
    for plan in plans {
        if !Artifacts::read(&plan)?.any() {
            continue;
        }
        let (report, error) = report(&plan, &SystemRunner).await?;
        reports.push(report);
        if let Some(error) = error {
            errors.push(format!("{error:#}"));
        }
    }
    println!(
        "{}",
        if reports.is_empty() {
            "🔴 安装状态：未安装".into()
        } else {
            reports.join("\n\n")
        }
    );
    anyhow::ensure!(
        errors.is_empty(),
        "无法确认系统服务状态：{}",
        errors.join("；")
    );
    Ok(())
}

async fn report(plan: &Plan, runner: &impl Runner) -> Result<(String, Option<anyhow::Error>)> {
    let files = Artifacts::read(plan)?;
    if !files.any() {
        return Ok(("🔴 安装状态：未安装".into(), None));
    }
    // A fresh checkout is not an installation and needs no running service manager.
    let state = if files.any() {
        Some(plan.manager.query(runner).await)
    } else {
        None
    };
    let registration_missing = matches!(&state, Some(Ok(state)) if !plan.manager.installed(state));
    let installed = if !files.complete() || registration_missing {
        "🔴 安装状态：安装不完整，请重新执行 install"
    } else if matches!(&state, Some(Err(_))) {
        "🔴 安装状态：无法确认"
    } else {
        "🟢 安装状态：已安装"
    };
    let mut lines = vec![
        installed.into(),
        format!("安装范围：{}", plan.scope.label()),
        format!("服务管理器：{}", plan.manager.label()),
        format!("服务名称：{}", plan.manager.name()),
        format!("当前 CLI 版本：{}", env!("CARGO_PKG_VERSION")),
    ];
    if files.executable {
        lines.push(format!("安装位置：{}", plan.executable.display()));
    }
    if files.definition {
        lines.push(format!("服务文件：{}", plan.definition.display()));
    }
    if files.complete() {
        let version = installed_version(plan, runner).await;
        lines.push(match version {
            Ok(version) => format!("已安装版本：{version}"),
            Err(error) => format!("已安装版本：未知（{error:#}）"),
        });
    }
    let mut query_error = None;
    match state {
        Some(Ok(state)) => {
            lines.push(format!(
                "系统注册/当前会话加载：{}",
                if state.registered { "是" } else { "否" }
            ));
            lines.push(format!(
                "运行状态：{}（{}）",
                if state.running {
                    "运行中"
                } else if state.pending {
                    "正在启动/停止"
                } else {
                    "未运行"
                },
                state.state
            ));
            if let Some(pid) = state.pid {
                lines.push(format!("PID：{pid}"));
            }
            if let Some(enabled) = state.enabled {
                lines.push(format!(
                    "自动启动/任务启用：{}",
                    if enabled { "已启用" } else { "未启用" }
                ));
            }
            if let Some(detail) = state.detail {
                lines.push(detail);
            }
        }
        Some(Err(error)) => {
            lines.push(format!("运行状态：未知（{error:#}）"));
            query_error = Some(error);
        }
        None => lines.push("运行状态：未安装，未查询系统服务".into()),
    }
    Ok((lines.join("\n"), query_error))
}

async fn installed_version(plan: &Plan, runner: &impl Runner) -> Result<String> {
    // Only --version is executed, so this never enters the daemon/login code path.
    let command = Invocation::new(&plan.executable, ["--version"]);
    let result = runner.output(&command).await?;
    result.check(&command)?;
    let version = result
        .stdout
        .trim()
        .strip_prefix(concat!(env!("CARGO_PKG_NAME"), " "))
        .filter(|value| {
            !value.is_empty()
                && value
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ".-+".contains(ch))
        })
        .context("安装副本未返回可识别的版本")?;
    Ok(version.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::testing::{FakeRunner, RUNNING};
    use std::fs;

    #[tokio::test]
    async fn system_reports_only_actual_artifacts_and_confirmed_registration() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let plan = crate::service::definition::linux_system_plan(
            dir.path().join("daemon"),
            dir.path().join("service"),
        )?;
        let runner = FakeRunner::default();
        assert_eq!(report(&plan, &runner).await?.0, "🔴 安装状态：未安装");
        assert!(runner.calls.borrow().is_empty());
        fs::write(&plan.definition, &plan.content)?;
        let runner = FakeRunner::new([(true, RUNNING)]);
        let (text, _) = report(&plan, &runner).await?;
        assert!(text.starts_with("🔴 安装状态：安装不完整，请重新执行 install\n安装范围：系统\n"));
        assert!(!text.contains("安装位置："));
        assert!(!text.contains("已安装版本："));
        fs::write(&plan.executable, "mock executable, never run")?;
        let runner = FakeRunner::new([(true, RUNNING), (true, "cxxy-wlanlogin-cli 0.0.9")]);
        assert!(
            report(&plan, &runner)
                .await?
                .0
                .starts_with("🟢 安装状态：已安装\n安装范围：系统\n")
        );
        let runner = FakeRunner::new([
            (false, "LoadState=not-found"),
            (true, "cxxy-wlanlogin-cli 0.0.9"),
        ]);
        assert!(
            report(&plan, &runner)
                .await?
                .0
                .starts_with("🔴 安装状态：安装不完整")
        );
        Ok(())
    }

    #[tokio::test]
    async fn missing_installation_status_runs_no_external_command() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let plan = crate::service::definition::linux_plan(
            dir.path().join("daemon"),
            dir.path().join("service"),
        )?;
        let runner = FakeRunner::default();
        let (text, error) = report(&plan, &runner).await?;
        assert_eq!(text, "🔴 安装状态：未安装");
        assert!(error.is_none());
        assert!(runner.calls.borrow().is_empty());
        assert!(!plan.definition.exists());
        Ok(())
    }

    #[tokio::test]
    async fn status_uses_installed_binary_version_and_system_manager_state() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let plan = crate::service::definition::linux_plan(
            dir.path().join("daemon"),
            dir.path().join("service"),
        )?;
        fs::write(&plan.definition, &plan.content)?;
        fs::write(&plan.executable, "mock executable, never run")?;
        let runner = FakeRunner::new([(true, RUNNING), (true, "cxxy-wlanlogin-cli 0.0.9\n")]);
        let (text, error) = report(&plan, &runner).await?;
        assert!(error.is_none());
        assert!(text.contains("安装状态：已安装"));
        assert!(text.starts_with("🟢 安装状态：已安装\n安装范围：用户\n"));
        assert!(text.contains("运行状态：运行中"));
        assert!(text.contains("已安装版本：0.0.9"));
        assert!(text.contains(&format!("当前 CLI 版本：{}", env!("CARGO_PKG_VERSION"))));
        assert!(text.contains("PID：123"));
        let calls = runner.calls.borrow();
        assert_eq!(calls[1].program, plan.executable.as_os_str());
        assert_eq!(calls[1].args, ["--version"]);
        Ok(())
    }

    #[tokio::test]
    async fn unavailable_manager_is_unknown_instead_of_stopped() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let plan = crate::service::definition::linux_plan(
            dir.path().join("daemon"),
            dir.path().join("service"),
        )?;
        fs::write(&plan.definition, &plan.content)?;
        fs::write(&plan.executable, "mock executable, never run")?;
        let runner = FakeRunner::new([
            (false, "Failed to connect to bus"),
            (false, "invalid executable"),
        ]);
        let (text, error) = report(&plan, &runner).await?;
        assert!(error.is_some());
        assert!(text.contains("运行状态：未知"));
        assert!(text.contains("已安装版本：未知"));
        assert!(!text.contains("运行状态：未运行"));
        Ok(())
    }
}
