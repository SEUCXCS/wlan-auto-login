use crate::infra::command::Invocation;
use crate::infra::error::Message;
use crate::infra::error::{Context, Result};
#[cfg(target_os = "macos")]
use crate::infra::identity::LAUNCHD_PLIST;
#[cfg(target_os = "macos")]
use crate::infra::paths::Paths;
use crate::system::define::services::{LABEL, Manager, Plan};
#[cfg(target_os = "macos")]
use crate::system::shared::services::artifacts::original_user_home;
use crate::system::shared::services::artifacts::xml;
use std::{ffi::OsStr, path::PathBuf};

pub(crate) fn macos_system_plan(
    executable: PathBuf,
    definition: PathBuf,
    log: PathBuf,
) -> Result<Plan> {
    let mut plan = macos_plan(executable, definition, log, 0)?;
    plan.scope = crate::infra::scope::Scope::System;
    plan.manager = Manager::Launchd {
        domain: "system".into(),
        target: format!("system/{LABEL}"),
    };
    plan.content = plan.content.replace(
        "<string>daemon</string>",
        "<string>--system</string><string>daemon</string>",
    );
    for invocation in plan
        .stop
        .iter_mut()
        .chain(&mut plan.start)
        .chain(&mut plan.remove)
    {
        for argument in &mut invocation.args {
            if let Some(value) = argument.to_str()
                && value.starts_with("gui/0")
            {
                *argument = value.replacen("gui/0", "system", 1).into();
            }
        }
    }
    Ok(plan)
}

pub(crate) fn macos_plan(
    executable: PathBuf,
    definition: PathBuf,
    log: PathBuf,
    uid: u32,
) -> Result<Plan> {
    let exe = xml(executable
        .to_str()
        .context(Message::InstallationPathMustBeUTF8)?);
    let log = xml(log.to_str().context(Message::LogPathMustBeUTF8)?);
    let domain = format!("gui/{uid}");
    let target = format!("{domain}/{LABEL}");
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LABEL}</string>
<key>ProgramArguments</key><array><string>{exe}</string><string>daemon</string></array>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
<key>ThrottleInterval</key><integer>30</integer>
<key>StandardErrorPath</key><string>{log}</string>
</dict></plist>
"#
    );
    let bootout =
        || Invocation::new("launchctl", [OsStr::new("bootout"), OsStr::new(&target)]).optional();
    Ok(Plan {
        scope: crate::infra::scope::Scope::User,
        manager: Manager::Launchd {
            domain: domain.clone(),
            target: target.clone(),
        },
        executable,
        content,
        stop: vec![bootout()],
        // Removing the plist prevents future loading; avoid leaving a persistent
        // launchd disabled override behind after uninstallation.
        remove: vec![bootout()],
        reload: vec![],
        start: vec![
            Invocation::new("launchctl", [OsStr::new("enable"), OsStr::new(&target)]),
            Invocation::new(
                "launchctl",
                [
                    OsStr::new("bootstrap"),
                    OsStr::new(&domain),
                    definition.as_os_str(),
                ],
            ),
        ],
        definition,
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn plan(paths: &Paths) -> Result<Plan> {
    if paths.scope == crate::infra::scope::Scope::System {
        return system_plan(paths);
    }
    let executable = crate::infra::paths::user_daemon_executable()?;

    {
        let base =
            directories::BaseDirs::new().context(Message::FailedToDetermineUserServiceDirectory)?;
        let uid = unsafe { libc::geteuid() };
        let log = paths.config.join("daemon.log");
        macos_plan(
            executable,
            base.home_dir()
                .join("Library/LaunchAgents")
                .join(LAUNCHD_PLIST),
            log,
            uid,
        )
    }
}
#[cfg(target_os = "macos")]
fn system_plan(paths: &Paths) -> Result<Plan> {
    {
        macos_system_plan(
            crate::infra::paths::system_daemon_executable()?,
            PathBuf::from("/Library/LaunchDaemons").join(LAUNCHD_PLIST),
            paths.config.join("daemon.log"),
        )
    }
}
#[cfg(target_os = "macos")]
pub(crate) fn conflicting_definition(paths: &Paths) -> Result<Option<PathBuf>> {
    use crate::infra::scope::Scope;
    let other = match paths.scope {
        Scope::User => system_plan(&Paths::new(Scope::System)?)?.definition,
        Scope::System => {
            if let Some(home) = original_user_home()? {
                home.join("Library/LaunchAgents").join(LAUNCHD_PLIST)
            } else {
                return Ok(None);
            }
        }
    };
    Ok(Some(other))
}
