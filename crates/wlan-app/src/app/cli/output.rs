//! Terminal rendering for controller results.
use crate::app::controller::login::LoginResult;
use crate::infra::error::Message;
use wlanlogin_lib::LoginOutcome;
pub fn login(result: &LoginResult) {
    if let Some(warning) = &result.warning {
        tracing::warn!("{warning}");
    }
    match result.outcome {
        Some(LoginOutcome::AlreadyOnline) => tracing::info!(
            credentials_saved = result.credentials_saved,
            "{}",
            Message::AlreadyOnline
        ),
        Some(LoginOutcome::Authenticated) => {
            tracing::info!(line = %result.line, credentials_saved = result.credentials_saved, "{}", Message::Authenticated)
        }
        None => tracing::info!("{}", Message::CredentialsSavedOffline),
    }
}

use crate::system::define::services::InstallationReport;
pub fn status(reports: &[InstallationReport]) -> String {
    if reports.is_empty() {
        "🔴 安装状态：未安装".into()
    } else {
        reports
            .iter()
            .map(format_report)
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}
fn format_report(report: &InstallationReport) -> String {
    if !report.files.any() {
        return "🔴 安装状态：未安装".into();
    }
    let installed = if !report.files.complete() || report.registration_missing {
        "🔴 安装状态：安装不完整，请重新执行 service install"
    } else if matches!(&report.state, Some(Err(_))) {
        "🔴 安装状态：无法确认"
    } else {
        "🟢 安装状态：已安装"
    };
    let mut lines = vec![
        installed.into(),
        format!("安装范围：{}", report.scope.label()),
        format!("服务管理器：{}", report.manager_label),
        format!("服务名称：{}", report.service_name),
        format!("当前 CLI 版本：{}", env!("CARGO_PKG_VERSION")),
    ];
    if report.files.executable {
        lines.push(format!("安装位置：{}", report.executable.display()));
    }
    if report.files.definition {
        lines.push(format!("服务文件：{}", report.definition.display()));
    }
    if let Some(version) = &report.version {
        lines.push(match version {
            Ok(version) => format!("已安装版本：{version}"),
            Err(error) => format!("已安装版本：未知（{error:#}）"),
        });
    }
    match &report.state {
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
            if let Some(detail) = &state.detail {
                lines.push(detail.clone());
            }
        }
        Some(Err(error)) => {
            lines.push(format!("运行状态：未知（{error:#}）"));
        }
        None => lines.push("运行状态：未安装，未查询系统服务".into()),
    }
    if let Some(Ok(runtime)) = &report.runtime {
        use crate::infra::control::CredentialState as S;
        let credentials = match runtime.credentials {
            S::NotChecked => "当前凭据尚未检查",
            S::Available => "可用",
            S::Missing => "未保存",
            S::WaitingForSession => "等待图形会话就绪",
            S::WaitingForProvider => "等待凭据服务",
            S::WaitingForUnlock => "等待钱包解锁",
            S::WaitingForPermission => "凭据访问被拒绝，请检查权限",
            S::WaitingForInteraction => "需要前台授权或解锁",
            S::Failed => "读取失败",
        };
        lines.push(format!("凭据状态：{credentials}"));
        if let Some(diagnostic) = runtime.diagnostic {
            lines.push(credential_diagnostic(diagnostic));
        }
        if let Some(outcome) = &runtime.outcome {
            lines.push(format!("自动登录状态：{outcome}"));
        }
    }
    if let Some(Err(error)) = &report.runtime {
        lines.push(format!("凭据状态：未知（无法获取 daemon 诊断：{error}）"));
    }
    lines.join("\n")
}
fn credential_diagnostic(diagnostic: crate::infra::error::credentials::Diagnostic) -> String {
    use crate::infra::error::credentials::{Kind, Stage};
    let message = match diagnostic.kind {
        Kind::Timeout => "凭据操作超时，稍后重试",
        Kind::InvalidEncoding | Kind::InvalidFormat => "保存的凭据格式损坏，请重新保存",
        Kind::UnsupportedVersion => "不支持已保存的凭据格式版本，请重新保存",
        Kind::Incomplete => "保存的凭据不完整，请重新保存",
        Kind::DuplicateEntries => "发现重复凭据，请处理本应用的重复条目",
        Kind::DecryptionFailed => "凭据解密失败，请检查存储环境或重新保存",
        Kind::UnsafeStorage => "凭据存储的路径或权限不符合要求，请检查存储",
        Kind::TaskFailed => "后台凭据读取任务失败，将重试",
        Kind::BackendUnavailable | Kind::ProviderUnavailable => {
            "凭据后端不可用，请检查服务或运行环境"
        }
        Kind::NativeFailure => "原生凭据操作失败，请检查诊断码",
        Kind::Unknown => "凭据操作失败，原因未识别，请查看日志",
        Kind::Locked => "钱包已锁定，请解锁",
        Kind::NeedsInteraction => "需要前台授权或解锁",
        Kind::PermissionDenied => "凭据访问被拒绝，请检查权限",
    };
    let stage = match diagnostic.stage {
        Stage::Session => "会话准备",
        Stage::Connect => "连接服务",
        Stage::Search => "查找条目",
        Stage::Inspect => "检查存储",
        Stage::Read => "读取凭据",
        Stage::Decode => "解析凭据",
        Stage::Decrypt => "解密",
        Stage::Task => "后台任务",
    };
    let mut detail = format!("凭据诊断：{message}（阶段：{stage}）");
    if let Some(code) = diagnostic.native_code {
        detail.push_str(&format!("；原生错误码：{code}"));
    }
    if let Some(code) = diagnostic.bus_code {
        detail.push_str(&format!("；D-Bus 错误：{code:?}"));
    }
    detail
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::controller::service::status::inspect;
    use crate::infra::{
        command::Runner,
        error::{Error, Result},
    };
    use crate::system::define::services::Plan;
    async fn report(plan: &Plan, runner: &impl Runner) -> Result<(String, Option<Error>)> {
        let report = inspect(plan, runner).await?;
        let text = format_report(&report);
        Ok((text, report.state.and_then(Result::err)))
    }
    use crate::infra::command::tests::{FakeRunner, RUNNING};
    use std::fs;

    #[test]
    fn credential_failures_have_distinct_recovery_messages_and_safe_native_codes() {
        use crate::infra::error::credentials::{Diagnostic, Kind, Stage};
        let timeout = credential_diagnostic(Diagnostic::new(Kind::Timeout, Stage::Read));
        let malformed = credential_diagnostic(Diagnostic::new(Kind::InvalidFormat, Stage::Decode));
        let duplicate =
            credential_diagnostic(Diagnostic::new(Kind::DuplicateEntries, Stage::Search));
        assert!(timeout.contains("超时") && !timeout.contains("权限"));
        assert!(malformed.contains("格式损坏") && malformed.contains("重新保存"));
        assert!(duplicate.contains("重复") && !duplicate.contains("解锁"));
        let mut native = Diagnostic::new(Kind::NativeFailure, Stage::Read);
        native.native_code = Some(-123);
        assert!(credential_diagnostic(native).contains("-123"));
    }

    #[tokio::test]
    async fn system_reports_only_actual_artifacts_and_confirmed_registration() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let plan = crate::system::shared::services::artifacts::linux_system_plan(
            dir.path().join("daemon"),
            dir.path().join("service"),
        )?;
        let runner = FakeRunner::default();
        assert_eq!(report(&plan, &runner).await?.0, "🔴 安装状态：未安装");
        assert!(runner.calls.borrow().is_empty());
        fs::write(&plan.definition, &plan.content)?;
        let runner = FakeRunner::new([(true, RUNNING)]);
        let (text, _) = report(&plan, &runner).await?;
        assert!(text.starts_with(
            "🔴 安装状态：安装不完整，请重新执行 service install\n安装范围：system\n"
        ));
        assert!(!text.contains("安装位置："));
        assert!(!text.contains("已安装版本："));
        fs::write(&plan.executable, "mock executable, never run")?;
        let runner = FakeRunner::new([(true, RUNNING), (true, "wlanlogin 0.0.9")]);
        assert!(
            report(&plan, &runner)
                .await?
                .0
                .starts_with("🟢 安装状态：已安装\n安装范围：system\n")
        );
        let runner = FakeRunner::new([(false, "LoadState=not-found"), (true, "wlanlogin 0.0.9")]);
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
        let plan = crate::system::shared::services::artifacts::linux_plan(
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
        let plan = crate::system::shared::services::artifacts::linux_plan(
            dir.path().join("daemon"),
            dir.path().join("service"),
        )?;
        fs::write(&plan.definition, &plan.content)?;
        fs::write(&plan.executable, "mock executable, never run")?;
        let runner = FakeRunner::new([(true, RUNNING), (true, "wlanlogin 0.0.9\n")]);
        let (text, error) = report(&plan, &runner).await?;
        assert!(error.is_none());
        assert!(text.contains("安装状态：已安装"));
        assert!(text.starts_with("🟢 安装状态：已安装\n安装范围：user\n"));
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
        let plan = crate::system::shared::services::artifacts::linux_plan(
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
