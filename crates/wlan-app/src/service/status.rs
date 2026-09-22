//! Report installation, running state and installed binary version.
use super::{
    command::{Invocation, Runner, SystemRunner},
    definition::{Artifacts, Plan, plan},
};
use crate::config::Paths;
use anyhow::{Context, Result};

pub async fn status(paths: &Paths, discover: bool) -> Result<()> {
    let mut plans = vec![plan(paths)?];
    if discover && paths.scope == crate::scope::Scope::User {
        plans.push(plan(&Paths::new(crate::scope::Scope::System)?)?);
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
mod tests;
