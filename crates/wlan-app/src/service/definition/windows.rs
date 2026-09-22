use super::{Invocation, LABEL, Manager, Plan, xml};
use anyhow::{Context, Result};
use std::{ffi::OsStr, path::PathBuf};

pub(in crate::service) fn windows_system_plan(
    executable: PathBuf,
    definition: PathBuf,
) -> Result<Plan> {
    let binary = format!(
        "\"{}\" --system daemon --service",
        executable.to_str().context("安装路径必须是 UTF-8")?
    );
    let stop = || Invocation::new("sc.exe", ["stop", "cxxy-wlanlogin"]).optional();
    Ok(Plan {
        scope: crate::scope::Scope::System,
        manager: Manager::WindowsService,
        content: serde_json::to_string_pretty(
            &serde_json::json!({"managed_by":LABEL,"scope":"system","executable":executable,"version":env!("CARGO_PKG_VERSION")}),
        )?,
        executable,
        definition,
        stop: vec![stop()],
        start: vec![
            Invocation::new(
                "sc.exe",
                [
                    "create",
                    "cxxy-wlanlogin",
                    "binPath=",
                    &binary,
                    "start=",
                    "auto",
                    "obj=",
                    "LocalSystem",
                ],
            )
            .optional(),
            Invocation::new(
                "sc.exe",
                [
                    "config",
                    "cxxy-wlanlogin",
                    "binPath=",
                    &binary,
                    "start=",
                    "auto",
                    "obj=",
                    "LocalSystem",
                ],
            ),
            Invocation::new(
                "sc.exe",
                [
                    "failure",
                    "cxxy-wlanlogin",
                    "reset=",
                    "86400",
                    "actions=",
                    "restart/30000/restart/60000/restart/300000",
                ],
            ),
            Invocation::new("sc.exe", ["failureflag", "cxxy-wlanlogin", "1"]),
            Invocation::new("sc.exe", ["start", "cxxy-wlanlogin"]),
        ],
        remove: vec![
            stop(),
            Invocation::new("sc.exe", ["delete", "cxxy-wlanlogin"]),
        ],
        reload: vec![],
    })
}

pub(in crate::service) fn windows_plan(
    executable: PathBuf,
    definition: PathBuf,
    sid: &str,
) -> Result<Plan> {
    let exe = xml(executable.to_str().context("安装路径必须是 UTF-8")?);
    let name = format!("cxxy-wlanlogin-{sid}");
    let sid = xml(sid);
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- {LABEL} -->
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
<RegistrationInfo><Description>Student_CX automatic login</Description></RegistrationInfo>
<Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{sid}</UserId></LogonTrigger></Triggers>
<Principals><Principal id="User"><UserId>{sid}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
<Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><StartWhenAvailable>true</StartWhenAvailable><RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable><AllowStartOnDemand>true</AllowStartOnDemand><Enabled>true</Enabled><Hidden>true</Hidden><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure></Settings>
<Actions Context="User"><Exec><Command>{exe}</Command><Arguments>daemon --background</Arguments></Exec></Actions>
</Task>
"#
    );
    Ok(Plan {
        scope: crate::scope::Scope::User,
        manager: Manager::TaskScheduler { name: name.clone() },
        executable,
        content,
        stop: vec![Invocation::new("schtasks", ["/End", "/TN", &name]).optional()],
        start: vec![
            Invocation::new(
                "schtasks",
                [
                    OsStr::new("/Create"),
                    OsStr::new("/TN"),
                    OsStr::new(&name),
                    OsStr::new("/XML"),
                    definition.as_os_str(),
                    OsStr::new("/F"),
                ],
            ),
            Invocation::new("schtasks", ["/Run", "/TN", &name]),
        ],
        remove: vec![
            Invocation::new("schtasks", ["/End", "/TN", &name]).optional(),
            Invocation::new("schtasks", ["/Delete", "/TN", &name, "/F"]),
        ],
        reload: vec![],
        definition,
    })
}
