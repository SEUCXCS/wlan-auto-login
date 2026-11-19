use super::{
    app_service,
    manager::{self, ConstraintRepair, LaunchState},
};
use crate::infra::{
    command::{Runner, SystemRunner},
    control,
    error::{Message, Result, ensure},
    paths::Paths,
};
use crate::system::define::services::{Manager, Plan, ServiceManager};
use std::{future::Future, time::Duration};

const READY_TIMEOUT: Duration = Duration::from_secs(20);
// The packaged jobs have a 30-second ThrottleInterval. The first readiness
// window has already elapsed when we begin watching the scheduled repair.
const CONSTRAINT_REPAIR_TIMEOUT: Duration = Duration::from_secs(30);

pub async fn wait_ready(paths: &Paths, plan: &Plan) -> Result<()> {
    ensure!(
        background_allowed(plan, &SystemRunner).await?,
        Message::MacosBackgroundApprovalRequired
    );
    if !wait_for_control(&mut || control::managed_running(paths)).await {
        return Err(not_ready(inspect(plan, &SystemRunner).await?));
    }
    check_payload(plan)
}

pub async fn wait_installed(paths: &Paths, plan: &Plan) -> Result<bool> {
    let ready = finish_install(plan, &SystemRunner, || control::managed_running(paths)).await?;
    if ready {
        check_payload(plan)?;
    }
    Ok(ready)
}

async fn finish_install<F: Future<Output = bool> + Send>(
    plan: &Plan,
    runner: &impl Runner,
    mut ready: impl FnMut() -> F + Send,
) -> Result<bool> {
    if !background_allowed(plan, runner).await? {
        return Ok(false);
    }
    if wait_for_control(&mut ready).await {
        return Ok(true);
    }

    let mut state = inspect(plan, runner).await?;
    if state.constraint_repair == Some(ConstraintRepair::Pending) {
        tracing::info!("{}", Message::WaitingForLaunchConstraintRepair);
        let repaired = tokio::time::timeout(CONSTRAINT_REPAIR_TIMEOUT, async {
            loop {
                if ready().await {
                    return Ok(true);
                }
                if state.constraint_repair == Some(ConstraintRepair::Failed) {
                    return Ok::<_, crate::infra::error::Error>(false);
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
                state = inspect(plan, runner).await?;
            }
        })
        .await;
        match repaired {
            Ok(Ok(true)) => return Ok(true),
            Ok(Ok(false)) => {}
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err(not_ready(state)),
        }
    }
    if state.constraint_repair != Some(ConstraintRepair::Failed) {
        return Err(not_ready(state));
    }

    // Service Management can replace the background item during repair while
    // the loaded job still uses the old item. Re-register the same signed app.
    tracing::warn!("{}", Message::ReregisteringAfterLaunchConstraintRepair);
    for command in &plan.start {
        app_service::output(command, runner).await?;
    }
    if !background_allowed(plan, runner).await? {
        return Ok(false);
    }
    if !wait_for_control(&mut ready).await {
        return Err(not_ready(inspect(plan, runner).await?));
    }
    Ok(true)
}

async fn wait_for_control<F: Future<Output = bool> + Send>(
    ready: &mut (impl FnMut() -> F + Send),
) -> bool {
    tokio::time::timeout(READY_TIMEOUT, async {
        loop {
            if ready().await {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .is_ok()
}

async fn background_allowed(plan: &Plan, runner: &impl Runner) -> Result<bool> {
    let state = plan.manager.query(runner).await?;
    if state.enabled == Some(false) {
        ensure!(state.registered, Message::InstallationIncomplete);
        return Ok(false);
    }
    Ok(true)
}

async fn inspect(plan: &Plan, runner: &impl Runner) -> Result<LaunchState> {
    let Manager::MacosAppService { domain, target, .. } = &plan.manager else {
        return Err(Message::UnexpectedServiceCommand.into());
    };
    manager::inspect(domain, target, runner).await
}

fn not_ready(state: LaunchState) -> crate::infra::error::Error {
    let detail = match state.service.detail {
        Some(detail) => format!(", {detail}"),
        None => String::new(),
    };
    Message::ManagedAppNotReady {
        state: state.service.state,
        detail,
    }
    .into()
}

fn check_payload(plan: &Plan) -> Result<()> {
    // User daemon startup creates its signed host before publishing control IPC.
    // A root service is ready even before GUI login: SSID must then wait for a user.
    ensure!(plan.executable.is_file(), Message::InstallationIncomplete);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::command::tests::FakeRunner;

    const ENABLED: &str =
        r#"{"version":1,"service":"dev.cxcs.tools.wlanlogin","scope":"user","status":"enabled"}"#;
    const LOADED: &str = "job = {\nstate = running\npid = 42\n}";
    const REPAIR_PENDING: &str = "job = {\nstate = spawn scheduled\nlast exit reason = OS_REASON_CODESIGNING\njob state = spawn failed\nproperties = needs LWCR update | has LWCR\n}";
    const REPAIR_FAILED: &str = "job = {\nstate = spawn scheduled\nlast exit code = 78: EX_CONFIG\njob state = spawn failed\nproperties = needs LWCR update | has LWCR\n}";
    const CONFIG_FAILED: &str = "job = {\nstate = spawn scheduled\nlast exit code = 78: EX_CONFIG\njob state = spawn failed\nproperties = has LWCR\n}";

    fn plan(dir: &std::path::Path) -> Result<Plan> {
        let plan = super::super::definition::app_service_plan(
            dir.join("wlanlogin.app/Contents/MacOS/wlanlogin"),
            false,
            501,
        )?;
        std::fs::create_dir_all(dir.join("wlanlogin.app/Contents/Library/LaunchAgents"))?;
        std::fs::write(&plan.definition, &plan.content)?;
        Ok(plan)
    }

    #[tokio::test(start_paused = true)]
    async fn installation_reregisters_once_after_launchd_repair_fails() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let plan = plan(dir.path())?;
        let runner = FakeRunner::new([
            (true, ENABLED),
            (true, REPAIR_PENDING),
            (true, REPAIR_PENDING),
            (true, REPAIR_FAILED),
            (true, ENABLED),
            (true, ENABLED),
            (true, LOADED),
        ]);
        let started = tokio::time::Instant::now();
        let ready =
            || std::future::ready(started.elapsed() >= READY_TIMEOUT + Duration::from_secs(2));
        assert!(finish_install(&plan, &runner, ready).await?);
        assert!(started.elapsed() >= READY_TIMEOUT + Duration::from_secs(1));
        let calls = runner.calls.borrow();
        assert_eq!(calls[4].args, ["--user", "service", "native", "register"]);
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.args.last().is_some_and(|arg| arg == "register"))
                .count(),
            1
        );
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn installation_reports_unrelated_spawn_failure_without_reregistering() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let plan = plan(dir.path())?;
        let runner = FakeRunner::new([
            (true, ENABLED),
            (true, CONFIG_FAILED),
            (true, CONFIG_FAILED),
        ]);
        let Err(error) = finish_install(&plan, &runner, || std::future::ready(false)).await else {
            panic!("an unready service should fail installation");
        };
        assert!(error.to_string().contains("Last exit code: 78: EX_CONFIG"));
        assert_eq!(runner.calls.borrow().len(), 3);
        Ok(())
    }
}
