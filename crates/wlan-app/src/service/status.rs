use super::{CommandOutput, Invocation, Plan, assert_owned, plan};
use crate::config::Paths;
use anyhow::{Context, Result};
use std::{collections::HashMap, fs, path::Path};

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

pub(super) trait Runner {
    fn output(
        &self,
        command: &Invocation,
    ) -> impl std::future::Future<Output = Result<CommandOutput>>;
}

pub(super) struct SystemRunner;
impl Runner for SystemRunner {
    async fn output(&self, command: &Invocation) -> Result<CommandOutput> {
        command.output().await
    }
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

pub(super) struct Artifacts {
    pub definition: bool,
    pub executable: bool,
}

impl Artifacts {
    pub fn read(plan: &Plan) -> Result<Self> {
        assert_owned(&plan.definition)?;
        Ok(Self {
            definition: is_file(&plan.definition)?,
            executable: is_file(&plan.executable)?,
        })
    }

    pub fn complete(&self) -> bool {
        self.definition && self.executable
    }

    fn any(&self) -> bool {
        self.definition || self.executable
    }
}

fn is_file(path: &Path) -> Result<bool> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("无法读取安装文件：{}", path.display())),
    }
}

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
    // A fresh checkout is not an installation and needs no running service manager.
    let state = if files.any() {
        Some(plan.manager.query(runner).await)
    } else {
        None
    };
    let registration_missing = matches!(&state, Some(Ok(state)) if !plan.manager.installed(state));
    let installed = if !files.any() {
        "未安装"
    } else if !files.complete() || registration_missing {
        "安装不完整，请重新执行 install"
    } else {
        "已安装"
    };
    let mut lines = vec![
        format!("安装状态：{installed}"),
        format!("服务管理器：{}", plan.manager.label()),
        format!("服务名称：{}", plan.manager.name()),
        format!(
            "程序位置：{}{}",
            plan.executable.display(),
            if files.executable {
                ""
            } else {
                "（不存在）"
            }
        ),
        format!(
            "服务文件：{}{}",
            plan.definition.display(),
            if files.definition {
                ""
            } else {
                "（不存在）"
            }
        ),
        format!("当前 CLI 版本：{}", env!("CARGO_PKG_VERSION")),
    ];
    if files.complete() {
        let version = installed_version(plan, runner).await;
        lines.push(match version {
            Ok(version) => format!("已安装版本：{version}"),
            Err(error) => format!("已安装版本：未知（{error:#}）"),
        });
    } else {
        lines.push("已安装版本：不可用".into());
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

impl Manager {
    fn label(&self) -> &'static str {
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
                    if launchd_domain_contains(&domain_result.stdout, super::LABEL) {
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
pub(super) mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque};

    #[derive(Default)]
    pub(in crate::service) struct FakeRunner {
        pub outputs: RefCell<VecDeque<CommandOutput>>,
        pub calls: RefCell<Vec<Invocation>>,
    }
    impl FakeRunner {
        pub fn new(outputs: impl IntoIterator<Item = (bool, &'static str)>) -> Self {
            Self {
                outputs: RefCell::new(
                    outputs
                        .into_iter()
                        .map(|(success, stdout)| CommandOutput {
                            success,
                            stdout: stdout.into(),
                            stderr: String::new(),
                        })
                        .collect(),
                ),
                calls: RefCell::new(Vec::new()),
            }
        }
    }
    impl Runner for FakeRunner {
        async fn output(&self, command: &Invocation) -> Result<CommandOutput> {
            self.calls.borrow_mut().push(command.clone());
            self.outputs
                .borrow_mut()
                .pop_front()
                .context("unexpected service command")
        }
    }

    pub const RUNNING: &str = "LoadState=loaded\nActiveState=active\nSubState=running\nUnitFileState=enabled\nMainPID=123\nResult=success\n";
    pub const STOPPED: &str = "LoadState=loaded\nActiveState=inactive\nSubState=dead\nUnitFileState=enabled\nMainPID=0\nResult=success\n";

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
    async fn missing_installation_status_runs_no_external_command() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let plan = super::super::linux_plan(dir.path().join("daemon"), dir.path().join("service"))?;
        let runner = FakeRunner::default();
        let (text, error) = report(&plan, &runner).await?;
        assert!(text.contains("安装状态：未安装"));
        assert!(error.is_none());
        assert!(runner.calls.borrow().is_empty());
        assert!(!plan.definition.exists());
        Ok(())
    }

    #[tokio::test]
    async fn status_uses_installed_binary_version_and_system_manager_state() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let plan = super::super::linux_plan(dir.path().join("daemon"), dir.path().join("service"))?;
        fs::write(&plan.definition, &plan.content)?;
        fs::write(&plan.executable, "mock executable, never run")?;
        let runner = FakeRunner::new([(true, RUNNING), (true, "cxxy-wlanlogin-cli 0.0.9\n")]);
        let (text, error) = report(&plan, &runner).await?;
        assert!(error.is_none());
        assert!(text.contains("安装状态：已安装"));
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
        let plan = super::super::linux_plan(dir.path().join("daemon"), dir.path().join("service"))?;
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
