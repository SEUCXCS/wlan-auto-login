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
