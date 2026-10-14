use crate::infra::error::Context;
use crate::infra::{
    command::{Invocation, Runner},
    error::{Message, Result},
};
use crate::system::define::services::{Action, Plan, ServiceManager, ServiceState};
use crate::system::shared::services::{fields, steps::execute};
pub(crate) async fn query(system: bool, name: &str, runner: &impl Runner) -> Result<ServiceState> {
    let command = Invocation::new(
        "systemctl",
        [
            if system { "--system" } else { "--user" },
            "show",
            name,
            "--no-pager",
            "--property=LoadState,ActiveState,SubState,UnitFileState,MainPID,Result",
        ],
    );
    let result = runner.output(&command).await?;
    if fields(&result.stdout).get("LoadState") != Some(&"not-found") {
        result.check(&command)?;
    }
    parse_systemd(&result.stdout)
}
pub(crate) async fn manage(
    system: bool,
    plan: &Plan,
    action: Action,
    runner: &impl Runner,
) -> Result<()> {
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
                if system { "--system" } else { "--user" },
                verb,
                plan.manager.name(),
            ],
        ),
    )
    .await
}
fn parse_systemd(text: &str) -> Result<ServiceState> {
    let values = fields(text);
    let load = values
        .get("LoadState")
        .context(Message::SystemdResponseIsMissingLoadState)?;
    if *load == "not-found" {
        return Ok(ServiceState::absent());
    }
    let active = values
        .get("ActiveState")
        .context(Message::SystemdResponseIsMissingActiveState)?;
    let sub = values
        .get("SubState")
        .context(Message::SystemdResponseIsMissingSubState)?;
    let pid = values
        .get("MainPID")
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|pid| *pid > 0);
    Ok(ServiceState {
        registered: true,
        running: matches!(*active, "active" | "reloading") && pid.is_some(),
        pending: matches!(*active, "activating" | "deactivating"),
        enabled: values
            .get("UnitFileState")
            .map(|value| matches!(*value, "enabled" | "enabled-runtime")),
        state: format!("{active}/{sub}; {load}"),
        pid,
        detail: values
            .get("Result")
            .filter(|value| !value.is_empty())
            .map(|value| {
                (Message::LastResultValue {
                    value: (value).to_string(),
                })
                .to_string()
            }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::command::tests::{RUNNING, STOPPED};
    #[test]
    fn systemd_distinguishes_running_stopped_failed_and_missing() -> Result<()> {
        let running = parse_systemd(RUNNING)?;
        assert!(running.running);
        assert_eq!(running.pid, Some(123));
        let stopped = parse_systemd(STOPPED)?;
        assert!(stopped.registered);
        assert!(!stopped.running);
        let failed = parse_systemd(
            "LoadState=loaded\nActiveState=failed\nSubState=failed\nMainPID=0\nResult=exit-code",
        )?;
        assert!(!failed.running);
        assert!(
            failed
                .detail
                .is_some_and(|detail| detail.contains("exit-code"))
        );
        assert!(!parse_systemd("LoadState=not-found")?.registered);
        assert!(parse_systemd("unrecognized output").is_err());
        Ok(())
    }
}
