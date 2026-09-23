use super::{Invocation, LABEL, Manager, Plan, xml};
use anyhow::{Context, Result};
use std::{ffi::OsStr, path::PathBuf};

pub(in crate::service) fn macos_system_plan(
    executable: PathBuf,
    definition: PathBuf,
    log: PathBuf,
) -> Result<Plan> {
    let mut plan = macos_plan(executable, definition, log, 0)?;
    plan.scope = crate::scope::Scope::System;
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

pub(in crate::service) fn macos_plan(
    executable: PathBuf,
    definition: PathBuf,
    log: PathBuf,
    uid: u32,
) -> Result<Plan> {
    let exe = xml(executable.to_str().context("安装路径必须是 UTF-8")?);
    let log = xml(log.to_str().context("日志路径必须是 UTF-8")?);
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
        scope: crate::scope::Scope::User,
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
