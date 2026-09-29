//! Query native service managers and normalize their state.
use super::command::{Invocation, Runner};
#[cfg(any(target_os = "linux", target_os = "macos", test))]
use anyhow::Context;
use anyhow::Result;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub(super) enum Manager {
    #[cfg(any(target_os = "linux", test))]
    Systemd { system: bool },
    #[cfg(any(target_os = "macos", test))]
    Launchd { domain: String, target: String },
    #[cfg(any(target_os = "windows", test))]
    TaskScheduler { name: String },
    #[cfg(any(windows, test))]
    WindowsService,
}

#[derive(Debug)]
pub(super) struct ServiceState {
    pub registered: bool,
    pub running: bool,
    pub pending: bool,
    pub enabled: Option<bool>,
    pub state: String,
    pub pid: Option<u32>,
    pub detail: Option<String>,
}

impl ServiceState {
    fn absent() -> Self {
        Self {
            registered: false,
            running: false,
            pending: false,
            enabled: None,
            state: "未注册/未加载".into(),
            pid: None,
            detail: None,
        }
    }
}

impl Manager {
    pub(super) fn label(&self) -> &'static str {
        match self {
            #[cfg(any(target_os = "linux", test))]
            Self::Systemd { system } => {
                if *system {
                    "systemd"
                } else {
                    "systemd --user"
                }
            }
            #[cfg(any(target_os = "macos", test))]
            Self::Launchd { domain, .. } if domain == "system" => "launchd LaunchDaemon",
            #[cfg(any(target_os = "macos", test))]
            Self::Launchd { .. } => "launchd LaunchAgent",
            #[cfg(any(target_os = "windows", test))]
            Self::TaskScheduler { .. } => "Windows Task Scheduler",
            #[cfg(any(windows, test))]
            Self::WindowsService => "Windows Service Control Manager",
        }
    }

    pub fn name(&self) -> &str {
        match self {
            #[cfg(any(target_os = "linux", test))]
            Self::Systemd { .. } => "cxxy-wlanlogin.service",
            #[cfg(any(target_os = "macos", test))]
            Self::Launchd { target, .. } => target,
            #[cfg(any(target_os = "windows", test))]
            Self::TaskScheduler { name } => name,
            #[cfg(any(windows, test))]
            Self::WindowsService => "cxxy-wlanlogin",
        }
    }

    pub fn installed(&self, state: &ServiceState) -> bool {
        match self {
            // bootout stops this login session; the installed plist loads next login.
            #[cfg(any(target_os = "macos", test))]
            Self::Launchd { .. } => {
                let _ = state;
                true
            }
            #[cfg(any(target_os = "linux", test))]
            Self::Systemd { .. } => state.registered,
            #[cfg(any(target_os = "windows", test))]
            Self::TaskScheduler { .. } => state.registered,
            #[cfg(any(windows, test))]
            Self::WindowsService => state.registered,
        }
    }

    pub async fn query(&self, runner: &impl Runner) -> Result<ServiceState> {
        match self {
            #[cfg(any(target_os = "linux", test))]
            Self::Systemd { system } => {
                let command = Invocation::new(
                    "systemctl",
                    [
                        if *system { "--system" } else { "--user" },
                        "show",
                        self.name(),
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
            #[cfg(any(target_os = "macos", test))]
            Self::Launchd { domain, target } => {
                let command = Invocation::new("launchctl", ["print", target]);
                let result = runner.output(&command).await?;
                if result.success {
                    parse_launchd(&result.stdout)
                } else {
                    // A missing job and an inaccessible GUI session are different.
                    let domain_query = Invocation::new("launchctl", ["print", domain]);
                    let domain_result = runner.output(&domain_query).await?;
                    domain_result.check(&domain_query)?;
                    if launchd_domain_contains(&domain_result.stdout, super::definition::LABEL) {
                        result.check(&command)?;
                    }
                    Ok(ServiceState::absent())
                }
            }
            #[cfg(any(target_os = "windows", test))]
            Self::TaskScheduler { name } => {
                let command = windows_query(name);
                let result = runner.output(&command).await?;
                result.check(&command)?;
                parse_windows(&result.stdout)
            }
            #[cfg(any(windows, test))]
            Self::WindowsService => {
                let command = windows_service_query();
                let result = runner.output(&command).await?;
                result.check(&command)?;
                parse_windows_service(&result.stdout)
            }
        }
    }
}

fn fields(text: &str) -> HashMap<&str, &str> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect()
}

#[cfg(any(target_os = "linux", test))]
fn parse_systemd(text: &str) -> Result<ServiceState> {
    let values = fields(text);
    let load = values
        .get("LoadState")
        .context("systemd 响应缺少 LoadState")?;
    if *load == "not-found" {
        return Ok(ServiceState::absent());
    }
    let active = values
        .get("ActiveState")
        .context("systemd 响应缺少 ActiveState")?;
    let sub = values
        .get("SubState")
        .context("systemd 响应缺少 SubState")?;
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
            .map(|value| format!("最近运行结果：{value}")),
    })
}

#[cfg(any(target_os = "macos", test))]
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
    let state = values.get("state").context("launchd 响应缺少 state")?;
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
        detail: values
            .get("last exit code")
            .map(|value| format!("最近退出代码：{value}")),
    })
}

#[cfg(any(target_os = "macos", test))]
fn launchd_domain_contains(text: &str, label: &str) -> bool {
    text.lines()
        .any(|line| line.split_whitespace().last() == Some(label))
}

#[cfg(any(target_os = "windows", test))]
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

#[cfg(any(target_os = "windows", test))]
fn parse_windows(text: &str) -> Result<ServiceState> {
    let values = fields(text);
    match values.get("Installed") {
        Some(&"0") => return Ok(ServiceState::absent()),
        Some(&"1") => {}
        _ => anyhow::bail!("计划任务响应缺少有效的 Installed 字段"),
    }
    let state = match values.get("State") {
        Some(&"0") => anyhow::bail!("计划任务状态未知"),
        Some(&"1") => "已禁用",
        Some(&"2") => "等待调度",
        Some(&"3") => "就绪",
        Some(&"4") => "运行中",
        _ => anyhow::bail!("计划任务响应缺少有效的 State 字段"),
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
        detail: values
            .get("LastTaskResult")
            .map(|value| format!("最近任务结果：{value}")),
    })
}

#[cfg(any(windows, test))]
fn windows_service_query() -> Invocation {
    let script = r#"$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding -ArgumentList $false
try {
    $service = Get-Service | Where-Object { $_.ServiceName -eq 'cxxy-wlanlogin' }
    if ($null -eq $service) { [Console]::WriteLine('Installed=0'); exit 0 }
    [Console]::WriteLine('Installed=1')
    [Console]::WriteLine('State=' + [int]$service.Status)
    $start = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Services\cxxy-wlanlogin').Start
    [Console]::WriteLine('Enabled=' + [int]($start -eq 2))
} catch { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }
"#;
    Invocation::new(
        "powershell.exe",
        [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ],
    )
}

#[cfg(any(windows, test))]
fn parse_windows_service(text: &str) -> Result<ServiceState> {
    let values = fields(text);
    match values.get("Installed") {
        Some(&"0") => return Ok(ServiceState::absent()),
        Some(&"1") => {}
        _ => anyhow::bail!("系统服务响应缺少 Installed"),
    }
    let (label, running, pending) = match values.get("State") {
        Some(&"1") => ("已停止", false, false),
        Some(&"2") => ("启动中", false, true),
        Some(&"3") => ("停止中", false, true),
        Some(&"4") => ("运行中", true, false),
        Some(&"5") | Some(&"6") => ("切换状态中", false, true),
        Some(&"7") => ("已暂停", false, false),
        _ => anyhow::bail!("未知的系统服务运行状态"),
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
    use crate::service::testing::{FakeRunner, RUNNING, STOPPED};

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

    #[tokio::test]
    async fn launchd_unloaded_job_requires_an_accessible_domain() -> Result<()> {
        let manager = Manager::Launchd {
            domain: "gui/501".into(),
            target: "gui/501/org.seucxcs.cxxy-wlanlogin".into(),
        };
        let runner = FakeRunner::new([
            (false, "missing job"),
            (true, "gui/501 = {\nservices = {\n}\n}"),
        ]);
        let state = manager.query(&runner).await?;
        assert!(!state.running);
        assert!(!state.registered);
        assert!(
            manager.installed(&state),
            "an unloaded plist remains installed"
        );
        let runner = FakeRunner::new([(false, "missing job"), (false, "missing domain")]);
        assert!(manager.query(&runner).await.is_err());
        Ok(())
    }
}
