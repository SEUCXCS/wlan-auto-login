use crate::infra::error::Context;
use crate::infra::{
    command::{Invocation, Runner},
    error::{Message, Result},
};
use crate::system::define::services::ServiceState;
use crate::system::shared::services::{fields, steps::execute};
pub(crate) async fn query(
    domain: &str,
    target: &str,
    runner: &impl Runner,
) -> Result<ServiceState> {
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
        Ok(ServiceState::absent())
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

fn parse_launchd(text: &str) -> Result<ServiceState> {
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
    Ok(ServiceState {
        registered: true,
        running: *state == "running" && pid.is_some(),
        pending: matches!(*state, "spawn scheduled" | "spawning"),
        enabled: None,
        state: (*state).into(),
        pid,
        detail: values.get("last exit code").map(|value| {
            (Message::LastExitCodeValue {
                value: (value).to_string(),
            })
            .to_string()
        }),
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
        )?;
        assert!(state.running);
        assert_eq!(state.pid, Some(987));
        assert!(!parse_launchd("job = {\nstate = not running\nlast exit code = 1\n}")?.running);
        assert!(parse_launchd("unrecognized output").is_err());
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
