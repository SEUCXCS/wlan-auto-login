use crate::infra::error::bail;
use crate::infra::{
    command::{Invocation, Runner},
    error::{Message, Result},
};
use crate::system::define::services::{Action, LABEL, Plan, ServiceState};
use crate::system::shared::services::lifecycle::wait_task_stopped;
use crate::system::shared::services::{fields, lifecycle::execute};
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
    let command = windows_service_query();
    let result = runner.output(&command).await?;
    result.check(&command)?;
    parse_windows_service(&result.stdout)
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
        if (($cause -is [System.Runtime.InteropServices.COMException]) -and ($cause.ErrorCode -eq -2147024894)) {{
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
    $start = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\{service}').Start
    [Console]::WriteLine('Enabled=' + [int]($start -eq 2))
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

    #[test]
    fn scm_registration_commands_and_query_share_the_service_name() -> Result<()> {
        let plan = crate::system::windows::services::definition::windows_system_plan(
            r"C:\Program Files\cxcs-tools\wlanlogin\wlanlogin-daemon.exe".into(),
            r"C:\ProgramData\cxcs-tools\wlanlogin\service.json".into(),
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
