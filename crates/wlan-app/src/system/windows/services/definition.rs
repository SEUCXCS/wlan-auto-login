use crate::infra::command::Invocation;
#[cfg(target_os = "windows")]
use crate::infra::paths::Paths;
use crate::system::define::services::{LABEL, Manager, Plan};
use crate::system::shared::services::artifacts::xml;

use crate::infra::error::Message;
use crate::infra::error::{Context, Result};
use std::{ffi::OsStr, path::PathBuf};

pub(crate) fn windows_system_plan(executable: PathBuf, definition: PathBuf) -> Result<Plan> {
    let binary = format!(
        "\"{}\" --system service run",
        executable
            .to_str()
            .context(Message::InstallationPathMustBeUTF8)?
    );
    let stop = || Invocation::new("sc.exe", ["stop", LABEL]).optional();
    Ok(Plan {
        scope: crate::infra::scope::Scope::System,
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
                    LABEL,
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
                    LABEL,
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
                    LABEL,
                    "reset=",
                    "86400",
                    "actions=",
                    "restart/30000/restart/60000/restart/300000",
                ],
            ),
            Invocation::new("sc.exe", ["failureflag", LABEL, "1"]),
            Invocation::new("sc.exe", ["start", LABEL]),
        ],
        remove: vec![stop(), Invocation::new("sc.exe", ["delete", LABEL])],
        reload: vec![],
    })
}

pub(crate) fn windows_plan(executable: PathBuf, definition: PathBuf, sid: &str) -> Result<Plan> {
    let exe = xml(executable
        .to_str()
        .context(Message::InstallationPathMustBeUTF8)?);
    let name = format!("{LABEL}-{sid}");
    let sid = xml(sid);
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!-- {LABEL} -->
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
<RegistrationInfo><Description>Student_CX automatic login</Description></RegistrationInfo>
<Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{sid}</UserId></LogonTrigger></Triggers>
<Principals><Principal id="User"><UserId>{sid}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
<Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><StartWhenAvailable>true</StartWhenAvailable><RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable><AllowStartOnDemand>true</AllowStartOnDemand><Enabled>true</Enabled><Hidden>true</Hidden><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure></Settings>
<Actions Context="User"><Exec><Command>{exe}</Command><Arguments>--user service run</Arguments></Exec></Actions>
</Task>
"#
    );
    Ok(Plan {
        scope: crate::infra::scope::Scope::User,
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

#[cfg(target_os = "windows")]
pub(crate) fn plan(paths: &Paths) -> Result<Plan> {
    if paths.scope == crate::infra::scope::Scope::System {
        return system_plan(paths);
    }
    let executable = crate::infra::paths::user_daemon_executable()?;

    {
        let sid = crate::infra::scope::current_sid()?;
        windows_plan(executable, paths.config.join("daemon-task.xml"), &sid)
    }
}
#[cfg(target_os = "windows")]
fn system_plan(paths: &Paths) -> Result<Plan> {
    {
        use windows_sys::Win32::UI::Shell::FOLDERID_ProgramData;
        let _ = paths;
        windows_system_plan(
            crate::infra::paths::system_daemon_executable()?,
            crate::infra::paths::app_dir(crate::infra::paths::known_folder(&FOLDERID_ProgramData)?)
                .join("service.json"),
        )
    }
}
#[cfg(target_os = "windows")]
pub(crate) fn conflicting_definition(paths: &Paths) -> Result<Option<PathBuf>> {
    use crate::infra::scope::Scope;
    let other = match paths.scope {
        Scope::User => system_plan(&Paths::new(Scope::System)?)?.definition,
        Scope::System => plan(&Paths::new(Scope::User)?)?.definition,
    };
    Ok(Some(other))
}
