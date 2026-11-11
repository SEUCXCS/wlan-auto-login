//! Native registration snapshots supplement the rollback already performed by the CLI.
use crate::{Scope, identity, win};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    os::windows::process::CommandExt,
    path::Path,
    process::Command,
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{ERROR_SERVICE_DOES_NOT_EXIST, GetLastError},
    System::Services::*,
};

#[derive(Serialize, Deserialize)]
pub struct Registration {
    pub running: bool,
    task: Option<String>,
    service: Option<Service>,
}

#[derive(Serialize, Deserialize)]
struct Service {
    binary: String,
    display: String,
    start: u32,
    error: u32,
    kind: u32,
    group: String,
    dependencies: Vec<u16>,
    account: String,
    reset: u32,
    reboot: String,
    command: String,
    actions: Vec<(i32, u32)>,
    non_crash: i32,
}
struct ServiceHandle(SC_HANDLE);
impl Drop for ServiceHandle {
    fn drop(&mut self) {
        unsafe {
            CloseServiceHandle(self.0);
        }
    }
}
fn manager(access: u32) -> Result<ServiceHandle> {
    let handle = unsafe { OpenSCManagerW(ptr::null(), ptr::null(), access) };
    ensure!(!handle.is_null(), "Could not open Service Control Manager");
    Ok(ServiceHandle(handle))
}
fn open(access: u32) -> Result<Option<ServiceHandle>> {
    let manager = manager(SC_MANAGER_CONNECT)?;
    let handle = unsafe { OpenServiceW(manager.0, win::wide(identity::APP_ID).as_ptr(), access) };
    if handle.is_null() {
        if unsafe { GetLastError() } == ERROR_SERVICE_DOES_NOT_EXIST {
            return Ok(None);
        }
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(Some(ServiceHandle(handle)))
}
fn service_state(handle: &ServiceHandle) -> Result<u32> {
    unsafe {
        let mut state: SERVICE_STATUS = std::mem::zeroed();
        win::check(QueryServiceStatus(handle.0, &mut state))?;
        Ok(state.dwCurrentState)
    }
}
fn query_config(handle: &ServiceHandle, level: Option<u32>) -> Result<Vec<usize>> {
    unsafe {
        let mut length = 0;
        if let Some(level) = level {
            QueryServiceConfig2W(handle.0, level, ptr::null_mut(), 0, &mut length);
        } else {
            QueryServiceConfigW(handle.0, ptr::null_mut(), 0, &mut length);
        }
        ensure!(length > 0, "Could not size native service configuration");
        let mut buffer = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
        let result = if let Some(level) = level {
            QueryServiceConfig2W(
                handle.0,
                level,
                buffer.as_mut_ptr().cast(),
                length,
                &mut length,
            )
        } else {
            QueryServiceConfigW(handle.0, buffer.as_mut_ptr().cast(), length, &mut length)
        };
        win::check(result)?;
        Ok(buffer)
    }
}

fn powershell(script: &str, scope: Scope) -> Result<String> {
    let name = format!("{}-{}", identity::APP_ID, win::sid()?);
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env("WLANLOGIN_SETUP_TASK", name)
        .env("WLANLOGIN_SETUP_SCOPE", scope.cli_flag())
        .creation_flags(0x08000000)
        .output()
        .context("Could not start Task Scheduler adapter")?;
    ensure!(
        output.status.success(),
        "Task Scheduler operation failed (exit {:?})",
        output.status.code()
    );
    String::from_utf8(output.stdout).context("Task Scheduler returned invalid text")
}
pub fn capture(scope: Scope) -> Result<Option<Registration>> {
    if scope == Scope::User {
        let text = powershell(
            r#"$ErrorActionPreference='Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding -ArgumentList $false
$s=New-Object -ComObject Schedule.Service; $s.Connect(); $f=$s.GetFolder('\')
$tasks=@($f.GetTasks(1) | Where-Object { $_.Name -eq $env:WLANLOGIN_SETUP_TASK })
if ($tasks.Count -eq 0) { [Console]::Write('null') } else {
 $t=$tasks[0]; [Console]::Write((@{xml=$t.Xml;running=($t.State -eq 4 -or $t.State -eq 2)} | ConvertTo-Json -Compress))
}"#,
            scope,
        )?;
        #[derive(Deserialize)]
        struct Task {
            xml: String,
            running: bool,
        }
        let task: Option<Task> = serde_json::from_str(text.trim())?;
        return task
            .map(|task| {
                ensure!(
                    task.xml.contains(identity::APP_ID) && task.xml.contains(&win::sid()?),
                    "Existing task ownership could not be verified"
                );
                Ok(Registration {
                    running: task.running,
                    task: Some(task.xml),
                    service: None,
                })
            })
            .transpose();
    }
    let Some(handle) = open(SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS)? else {
        return Ok(None);
    };
    let state = service_state(&handle)?;
    ensure!(
        [SERVICE_RUNNING, SERVICE_STOPPED].contains(&state),
        "Existing service must reach a stable running or stopped state"
    );
    unsafe {
        let buffer = query_config(&handle, None)?;
        let config = &*buffer.as_ptr().cast::<QUERY_SERVICE_CONFIGW>();
        let binary = win::string_from_ptr(config.lpBinaryPathName);
        let account = win::string_from_ptr(config.lpServiceStartName);
        ensure!(
            binary.contains("wlanlogin-daemon.exe") && account.eq_ignore_ascii_case("LocalSystem"),
            "Existing service ownership or account could not be verified"
        );
        let mut dependencies = vec![0, 0];
        if !config.lpDependencies.is_null() && *config.lpDependencies != 0 {
            let mut size = 0;
            while *config.lpDependencies.add(size) != 0 || *config.lpDependencies.add(size + 1) != 0
            {
                size += 1;
            }
            dependencies = std::slice::from_raw_parts(config.lpDependencies, size + 2).to_vec();
        }
        let failure_buffer = query_config(&handle, Some(SERVICE_CONFIG_FAILURE_ACTIONS))?;
        let failures = &*failure_buffer.as_ptr().cast::<SERVICE_FAILURE_ACTIONSW>();
        let mut actions = Vec::new();
        for index in 0..failures.cActions as usize {
            let action = &*failures.lpsaActions.add(index);
            actions.push((action.Type, action.Delay));
        }
        let flag_buffer = query_config(&handle, Some(SERVICE_CONFIG_FAILURE_ACTIONS_FLAG))?;
        let non_crash = (*flag_buffer.as_ptr().cast::<SERVICE_FAILURE_ACTIONS_FLAG>())
            .fFailureActionsOnNonCrashFailures;
        Ok(Some(Registration {
            running: state == SERVICE_RUNNING,
            task: None,
            service: Some(Service {
                binary,
                account,
                display: win::string_from_ptr(config.lpDisplayName),
                start: config.dwStartType,
                error: config.dwErrorControl,
                kind: config.dwServiceType,
                group: win::string_from_ptr(config.lpLoadOrderGroup),
                dependencies,
                reset: failures.dwResetPeriod,
                reboot: win::string_from_ptr(failures.lpRebootMsg),
                command: win::string_from_ptr(failures.lpCommand),
                actions,
                non_crash,
            }),
        }))
    }
}

impl Registration {
    pub fn restore(&self, scope: Scope, directory: &Path) -> Result<()> {
        if let Some(xml) = &self.task {
            let file = directory.join("task.xml");
            std::fs::write(&file, xml.as_bytes())?;
            let script = r#"$ErrorActionPreference='Stop'
$s=New-Object -ComObject Schedule.Service; $s.Connect(); $f=$s.GetFolder('\')
$xml=[IO.File]::ReadAllText($env:WLANLOGIN_SETUP_XML)
$t=$f.RegisterTask($env:WLANLOGIN_SETUP_TASK,$xml,6,$null,$null,3,$null)
if ($env:WLANLOGIN_SETUP_RUNNING -eq '1') { $null=$t.Run($null) }
"#;
            let output = Command::new("powershell.exe")
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    script,
                ])
                .env("WLANLOGIN_SETUP_XML", file)
                .env(
                    "WLANLOGIN_SETUP_TASK",
                    format!("{}-{}", identity::APP_ID, win::sid()?),
                )
                .env(
                    "WLANLOGIN_SETUP_RUNNING",
                    if self.running { "1" } else { "0" },
                )
                .creation_flags(0x08000000)
                .output()?;
            ensure!(
                output.status.success(),
                "Could not restore original scheduled task"
            );
        }
        if let Some(service) = &self.service {
            unsafe {
                let handle = if let Some(handle) = open(SERVICE_ALL_ACCESS)? {
                    handle
                } else {
                    let manager = manager(SC_MANAGER_CREATE_SERVICE)?;
                    let handle = CreateServiceW(
                        manager.0,
                        win::wide(identity::APP_ID).as_ptr(),
                        win::wide(&service.display).as_ptr(),
                        SERVICE_ALL_ACCESS,
                        service.kind,
                        service.start,
                        service.error,
                        win::wide(&service.binary).as_ptr(),
                        ptr::null(),
                        ptr::null_mut(),
                        service.dependencies.as_ptr(),
                        win::wide(&service.account).as_ptr(),
                        ptr::null(),
                    );
                    ensure!(!handle.is_null(), "Could not recreate the original service");
                    ServiceHandle(handle)
                };
                win::check(ChangeServiceConfigW(
                    handle.0,
                    service.kind,
                    service.start,
                    service.error,
                    win::wide(&service.binary).as_ptr(),
                    win::wide(&service.group).as_ptr(),
                    ptr::null_mut(),
                    service.dependencies.as_ptr(),
                    win::wide(&service.account).as_ptr(),
                    ptr::null(),
                    win::wide(&service.display).as_ptr(),
                ))?;
                let mut actions: Vec<_> = service
                    .actions
                    .iter()
                    .map(|(kind, delay)| SC_ACTION {
                        Type: *kind,
                        Delay: *delay,
                    })
                    .collect();
                let reboot = win::wide(&service.reboot);
                let command = win::wide(&service.command);
                let failures = SERVICE_FAILURE_ACTIONSW {
                    dwResetPeriod: service.reset,
                    lpRebootMsg: reboot.as_ptr().cast_mut(),
                    lpCommand: command.as_ptr().cast_mut(),
                    cActions: actions.len() as u32,
                    lpsaActions: actions.as_mut_ptr(),
                };
                win::check(ChangeServiceConfig2W(
                    handle.0,
                    SERVICE_CONFIG_FAILURE_ACTIONS,
                    (&failures as *const SERVICE_FAILURE_ACTIONSW).cast(),
                ))?;
                let flags = SERVICE_FAILURE_ACTIONS_FLAG {
                    fFailureActionsOnNonCrashFailures: service.non_crash,
                };
                win::check(ChangeServiceConfig2W(
                    handle.0,
                    SERVICE_CONFIG_FAILURE_ACTIONS_FLAG,
                    (&flags as *const SERVICE_FAILURE_ACTIONS_FLAG).cast(),
                ))?;
                if self.running {
                    win::check(StartServiceW(handle.0, 0, ptr::null()))?;
                }
            }
        }
        wait(scope, self.running)
    }
}

pub fn wait(scope: Scope, running: bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let actual = if scope == Scope::Machine {
            match open(SERVICE_QUERY_STATUS)? {
                Some(handle) => service_state(&handle)? == SERVICE_RUNNING,
                None => false,
            }
        } else {
            capture(scope)?.is_some_and(|registration| registration.running)
        };
        if actual == running {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "Timed out waiting for background runtime state"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}
