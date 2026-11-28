use crate::infra::error::bail;
use crate::infra::{
    command::{Invocation, Runner},
    error::{Message, Result},
};
use crate::system::define::services::{Action, LABEL, Plan, ServiceState};
use crate::system::shared::services::{fields, steps::execute};
use crate::system::windows::services::wait::wait_task_stopped;
pub(crate) async fn query_task(name: &str, runner: &impl Runner) -> Result<ServiceState> {
    let command = windows_query(name);
    let result = runner.output(&command).await?;
    result.check(&command)?;
    parse_windows(&result.stdout)
}
pub(crate) async fn manage_task(
    name: &str,
    plan: &Plan,
    action: Action,
    state: ServiceState,
    runner: &impl Runner,
) -> Result<()> {
    let mut state = state;
    if matches!(action, Action::Stop | Action::Restart) && (state.running || state.pending) {
        execute(runner, Invocation::new("schtasks", ["/End", "/TN", name])).await?;
        state = wait_task_stopped(&plan.manager, runner).await?;
    }
    if matches!(action, Action::Start | Action::Restart) && !state.running && !state.pending {
        if !state.registered {
            bail!(Message::ServiceRegistrationRemoved);
        }
        execute(runner, Invocation::new("schtasks", ["/Run", "/TN", name])).await?;
    }
    Ok(())
}
pub(crate) async fn query_service(runner: &impl Runner) -> Result<ServiceState> {
    parse_windows_service(&service_properties(runner).await?)
}
async fn service_properties(runner: &impl Runner) -> Result<String> {
    let command = windows_service_query();
    let result = runner.output(&command).await?;
    result.check(&command)?;
    Ok(result.stdout)
}

#[cfg(target_os = "windows")]
pub(crate) struct ScmRegistration {
    state: ServiceState,
    start_type: Option<&'static str>,
}

#[cfg(target_os = "windows")]
impl ScmRegistration {
    pub(crate) async fn capture(runner: &impl Runner) -> Result<Self> {
        let properties = service_properties(runner).await?;
        let state = parse_windows_service(&properties)?;
        let values = fields(&properties);
        let start_type = if state.registered {
            Some(match values.get("StartType") {
                Some(&"2") if values.get("Delayed") == Some(&"1") => "delayed-auto",
                Some(&"2") => "auto",
                Some(&"3") => "demand",
                Some(&"4") => "disabled",
                _ => bail!("SCM returned an unsupported start type"),
            })
        } else {
            None
        };
        Ok(Self { state, start_type })
    }

    pub(crate) async fn restore(&self, plan: &Plan, runner: &impl Runner) -> Result<()> {
        let Some(start_type) = self.start_type else {
            return Ok(());
        };
        for mut command in plan.start.iter().cloned() {
            if command.args.first().is_some_and(|verb| verb == "start")
                && !self.state.running
                && self.state.state != "starting"
            {
                continue;
            }
            if let Some(index) = command
                .args
                .iter()
                .position(|argument| argument == "start=")
            {
                let Some(argument) = command.args.get_mut(index + 1) else {
                    bail!("SCM registration command is missing its start type");
                };
                *argument = start_type.into();
            }
            crate::system::shared::services::steps::execute_optional(runner, &command).await?;
        }
        Ok(())
    }
}
pub(crate) async fn manage_service(
    plan: &Plan,
    action: Action,
    state: ServiceState,
    runner: &impl Runner,
) -> Result<()> {
    let mut state = state;
    if matches!(action, Action::Stop | Action::Restart) && (state.running || state.pending) {
        execute(runner, Invocation::new("sc.exe", ["stop", LABEL])).await?;
        state = wait_task_stopped(&plan.manager, runner).await?;
    }
    if matches!(action, Action::Start | Action::Restart) && !state.running && !state.pending {
        crate::infra::error::ensure!(state.registered, Message::ServiceRegistrationRemoved);
        execute(runner, Invocation::new("sc.exe", ["start", LABEL])).await?;
    }
    Ok(())
}
fn windows_query(name: &str) -> Invocation {
    // Task Scheduler's COM API and PowerShell 2 exist on Win7. Numeric states
    // avoid parsing localized schtasks output; no ConvertTo-Json (PS3+) is used.
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding -ArgumentList $false
try {{
    $scheduler = New-Object -ComObject 'Schedule.Service'
    $scheduler.Connect()
    $folder = $scheduler.GetFolder('\')
    try {{ $task = $folder.GetTask('{name}') }} catch {{
        $cause = $_.Exception
        while ($null -ne $cause.InnerException) {{ $cause = $cause.InnerException }}
        if ([System.Runtime.InteropServices.Marshal]::GetHRForException($cause) -eq -2147024894) {{
            [Console]::WriteLine('Installed=0')
            exit 0
        }}
        throw
    }}
    [Console]::WriteLine('Installed=1')
    [Console]::WriteLine('State=' + [int]$task.State)
    [Console]::WriteLine('Enabled=' + [int][bool]$task.Enabled)
    [Console]::WriteLine('LastTaskResult=' + [long]$task.LastTaskResult)
}} catch {{
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}}
"#,
        name = name.replace('\'', "''")
    );
    Invocation::new(
        "powershell.exe",
        [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ],
    )
}
fn parse_windows(text: &str) -> Result<ServiceState> {
    let values = fields(text);
    match values.get("Installed") {
        Some(&"0") => return Ok(ServiceState::absent()),
        Some(&"1") => {}
        _ => crate::infra::error::bail!(Message::TaskSchedulerResponseHasAnInvalidInstalledField),
    }
    let state = match values.get("State") {
        Some(&"0") => {
            crate::infra::error::bail!(Message::UnknownTaskSchedulerState)
        }
        Some(&"1") => "disabled",
        Some(&"2") => "queued",
        Some(&"3") => "ready",
        Some(&"4") => "running",
        _ => crate::infra::error::bail!(Message::TaskSchedulerResponseHasAnInvalidStateField),
    };
    Ok(ServiceState {
        registered: true,
        running: values.get("State") == Some(&"4"),
        pending: values.get("State") == Some(&"2"),
        enabled: match values.get("Enabled") {
            Some(&"0") => Some(false),
            Some(&"1") => Some(true),
            _ => None,
        },
        state: state.into(),
        pid: None,
        detail: values.get("LastTaskResult").map(|value| {
            (Message::LastTaskResultValue {
                value: (value).to_string(),
            })
            .to_string()
        }),
    })
}
fn windows_service_query() -> Invocation {
    let script = r#"$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding -ArgumentList $false
try {
    $service = Get-Service | Where-Object { $_.ServiceName -eq '{service}' }
    if ($null -eq $service) { [Console]::WriteLine('Installed=0'); exit 0 }
    [Console]::WriteLine('Installed=1')
    [Console]::WriteLine('State=' + [int]$service.Status)
    $config = Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\{service}'
    $start = $config.Start
    [Console]::WriteLine('Enabled=' + [int]($start -eq 2))
    [Console]::WriteLine('StartType=' + [int]$start)
    [Console]::WriteLine('Delayed=' + [int]($config.DelayedAutoStart -eq 1))
} catch { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }
"#
    .replace("{service}", LABEL);
    Invocation::new(
        "powershell.exe",
        [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &script,
        ],
    )
}
fn parse_windows_service(text: &str) -> Result<ServiceState> {
    let values = fields(text);
    match values.get("Installed") {
        Some(&"0") => return Ok(ServiceState::absent()),
        Some(&"1") => {}
        _ => {
            crate::infra::error::bail!(Message::SCMResponseIsMissingInstalled)
        }
    }
    let (label, running, pending) = match values.get("State") {
        Some(&"1") => ("stopped", false, false),
        Some(&"2") => ("starting", false, true),
        Some(&"3") => ("stopping", false, true),
        Some(&"4") => ("running", true, false),
        Some(&"5") | Some(&"6") => ("pending", false, true),
        Some(&"7") => ("paused", false, false),
        _ => crate::infra::error::bail!(Message::UnknownSCMServiceState),
    };
    Ok(ServiceState {
        registered: true,
        running,
        pending,
        state: label.into(),
        enabled: values.get("Enabled").map(|v| *v == "1"),
        pid: None,
        detail: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::error::Context;
    use crate::system::define::services::ServiceManager;

    #[cfg(windows)]
    #[tokio::test]
    async fn scm_rollback_preserves_a_disabled_stopped_service() -> Result<()> {
        let runner = crate::infra::command::tests::FakeRunner::new([
            (
                true,
                "Installed=1\nState=1\nEnabled=0\nStartType=4\nDelayed=0",
            ),
            (true, ""),
            (true, ""),
            (true, ""),
            (true, ""),
        ]);
        let plan = crate::system::windows::services::definition::windows_system_plan(
            "daemon.exe".into(),
            "service.json".into(),
        )?;
        ScmRegistration::capture(&runner)
            .await?
            .restore(&plan, &runner)
            .await?;
        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 5);
        assert!(calls[1].args.iter().any(|argument| argument == "disabled"));
        assert!(calls[2].args.iter().any(|argument| argument == "disabled"));
        assert!(
            calls
                .iter()
                .all(|call| call.args.first().is_none_or(|argument| argument != "start"))
        );
        Ok(())
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn missing_task_is_absent_in_windows_powershell() -> Result<()> {
        let name = format!(
            "wlanlogin-missing-task-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .context("system clock is before the Unix epoch")?
                .as_nanos()
        );
        let state = query_task(&name, &crate::infra::command::SystemRunner).await?;
        assert!(!state.registered);
        Ok(())
    }

    #[test]
    fn scm_registration_commands_and_query_share_the_service_name() -> Result<()> {
        let plan = crate::system::windows::services::definition::windows_system_plan(
            r"C:\Program Files\SEUCXCS\wlanlogin\wlanlogin-daemon.exe".into(),
            r"C:\ProgramData\SEUCXCS\wlanlogin\service.json".into(),
        )?;
        let name = plan.manager.name();
        assert_eq!(name, "dev.cxcs.tools.wlanlogin");
        for command in plan.start.iter().chain(&plan.stop).chain(&plan.remove) {
            assert_eq!(command.program, "sc.exe");
            assert_eq!(command.args[1], name);
        }
        let query = windows_service_query();
        let script = query
            .args
            .last()
            .context("missing script")?
            .to_string_lossy();
        assert!(script.contains(&format!("$_.ServiceName -eq '{name}'")));
        assert!(script.contains(&format!(r"HKLM:\SYSTEM\CurrentControlSet\Services\{name}")));
        assert!(!script.contains("{service}"));
        Ok(())
    }

    #[test]
    fn windows_numeric_states_do_not_depend_on_system_language() -> Result<()> {
        for number in 1..=4 {
            let state = parse_windows(&format!(
                "Installed=1\r\nState={number}\r\nEnabled=1\r\nLastTaskResult=0\r\n"
            ))?;
            assert!(state.registered);
            assert_eq!(state.running, number == 4);
        }
        assert!(!parse_windows("Installed=0")?.registered);
        assert!(parse_windows("Installed=1\nState=0").is_err());
        assert!(parse_windows("Installed=1\nState=5").is_err());
        assert!(parse_windows("access denied").is_err());
        let command = windows_query("task'with-quote");
        assert_eq!(command.program, "powershell.exe");
        let script = command
            .args
            .last()
            .context("missing script")?
            .to_string_lossy();
        assert!(script.contains("task''with-quote"));
        assert!(!script.contains("ConvertTo-Json"));
        Ok(())
    }
    #[test]
    fn scm_numeric_states_are_independent_of_locale() -> Result<()> {
        for number in 1..=7 {
            let state = parse_windows_service(&format!("Installed=1\nState={number}\nEnabled=1"))?;
            assert!(state.registered);
            assert_eq!(state.running, number == 4);
            assert_eq!(state.pending, matches!(number, 2 | 3 | 5 | 6));
        }
        assert!(!parse_windows_service("Installed=0")?.registered);
        assert!(parse_windows_service("Installed=1\nState=unknown").is_err());
        assert!(parse_windows_service("access denied").is_err());
        Ok(())
    }
}
