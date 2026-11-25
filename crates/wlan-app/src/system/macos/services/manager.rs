use crate::infra::error::Context;
use crate::infra::{
    command::{Invocation, Runner},
    error::{Message, Result},
};
use crate::system::define::services::ServiceState;
use crate::system::shared::services::{fields, steps::execute};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ConstraintRepair {
    Pending,
    Failed,
}

pub(super) struct LaunchState {
    pub service: ServiceState,
    pub constraint_repair: Option<ConstraintRepair>,
}

pub(crate) async fn query(
    domain: &str,
    target: &str,
    runner: &impl Runner,
) -> Result<ServiceState> {
    Ok(inspect(domain, target, runner).await?.service)
}

pub(super) async fn inspect(
    domain: &str,
    target: &str,
    runner: &impl Runner,
) -> Result<LaunchState> {
    let command = Invocation::new("launchctl", ["print", target]);
    let result = runner.output(&command).await?;
    if result.success {
        parse_launchd(&result.stdout)
    } else {
        // A missing job and an inaccessible GUI session are different.
        let domain_query = Invocation::new("launchctl", ["print", domain]);
        let domain_result = runner.output(&domain_query).await?;
        domain_result.check(&domain_query)?;
        if launchd_domain_contains(
            &domain_result.stdout,
            crate::system::define::services::LABEL,
        ) {
            result.check(&command)?;
        }
        Ok(LaunchState {
            service: ServiceState::absent(),
            constraint_repair: None,
        })
    }
}
pub(crate) async fn stop(target: &str, state: ServiceState, runner: &impl Runner) -> Result<()> {
    if state.registered {
        execute(runner, Invocation::new("launchctl", ["bootout", target])).await?;
    }
    Ok(())
}

pub(crate) async fn start(target: &str, runner: &impl Runner) -> Result<()> {
    execute(runner, Invocation::new("launchctl", ["kickstart", target])).await
}

fn parse_launchd(text: &str) -> Result<LaunchState> {
    // Only parse the job's own properties, not nested resource/exit dictionaries.
    let mut depth = 0usize;
    let mut properties = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.ends_with('{') {
            depth += 1;
        } else if line == "}" {
            depth = depth.saturating_sub(1);
        } else if depth == 1 {
            properties.push_str(line);
            properties.push('\n');
        }
    }
    let values = fields(&properties);
    let state = values
        .get("state")
        .context(Message::LaunchdResponseIsMissingState)?;
    let pid = values
        .get("pid")
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|pid| *pid > 0);
    let exit_reason = values.get("last exit reason").copied();
    let exit_code = values.get("last exit code").copied();
    let constraint_repair = if values.get("properties").is_some_and(|value| {
        value
            .split(" | ")
            .any(|property| property == "needs LWCR update")
    }) {
        match (exit_reason, values.get("job state").copied(), exit_code) {
            (Some("OS_REASON_CODESIGNING"), _, _) => Some(ConstraintRepair::Pending),
            (_, Some("spawn failed"), Some("78: EX_CONFIG")) => Some(ConstraintRepair::Failed),
            _ => None,
        }
    } else {
        None
    };
    let detail = exit_reason
        .map(|reason| Message::LastExitReasonValue {
            value: reason.into(),
        })
        .or_else(|| exit_code.map(|code| Message::LastExitCodeValue { value: code.into() }))
        .map(|message| message.to_string());
    let service = ServiceState {
        registered: true,
        running: *state == "running" && pid.is_some(),
        pending: matches!(*state, "spawn scheduled" | "spawning"),
        enabled: None,
        state: (*state).into(),
        pid,
        detail,
    };
    Ok(LaunchState {
        service,
        constraint_repair,
    })
}
fn launchd_domain_contains(text: &str, label: &str) -> bool {
    text.lines()
        .any(|line| line.split_whitespace().last() == Some(label))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::command::tests::FakeRunner;
    #[test]
    fn launchd_reads_only_top_level_job_properties() -> Result<()> {
        let state = parse_launchd(
            "gui/501/example = {\nstate = running\npid = 987\nenvironment = {\nstate = stopped\npid = 0\n}\nlast exit code = 0\n}",
        )?.service;
        assert!(state.running);
        assert_eq!(state.pid, Some(987));
        assert!(
            !parse_launchd("job = {\nstate = not running\nlast exit code = 1\n}")?
                .service
                .running
        );
        assert!(parse_launchd("unrecognized output").is_err());
        Ok(())
    }

    #[test]
    fn launchd_reports_signing_repair_only_for_matching_spawn_failures() -> Result<()> {
        let samples = [
            (
                "last exit reason = OS_REASON_CODESIGNING",
                "has LWCR | needs LWCR update",
                Some(ConstraintRepair::Pending),
            ),
            (
                "last exit code = 78: EX_CONFIG",
                "has LWCR | needs LWCR update",
                Some(ConstraintRepair::Failed),
            ),
            ("last exit code = 78: EX_CONFIG", "has LWCR", None),
        ];
        for (exit, properties, repair) in samples {
            let state = parse_launchd(&format!(
                "job = {{\nstate = spawn scheduled\n{exit}\njob state = spawn failed\nproperties = {properties}\n}}"
            ))?;
            assert_eq!(state.constraint_repair, repair);
            assert!(state.service.detail.is_some());
        }
        Ok(())
    }
    #[tokio::test]
    async fn launchd_unloaded_job_requires_an_accessible_domain() -> Result<()> {
        let domain = "gui/501";
        let target = "gui/501/dev.cxcs.tools.wlanlogin";
        let runner = FakeRunner::new([
            (false, "missing job"),
            (true, "gui/501 = {\nservices = {\n}\n}"),
        ]);
        let state = query(domain, target, &runner).await?;
        assert!(!state.running);
        assert!(!state.registered);
        let runner = FakeRunner::new([(false, "missing job"), (false, "missing domain")]);
        assert!(query(domain, target, &runner).await.is_err());
        Ok(())
    }
}
