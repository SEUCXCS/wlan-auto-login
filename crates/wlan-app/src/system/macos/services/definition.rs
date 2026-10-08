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
    let arguments = if uid == 0 {
        format!(
            "<string>{exe}</string><string>--system</string><string>daemon</string><string>--background</string>"
        )
    } else {
        // Launch Services supplies the bundle identity used by TCC. -W keeps
        // launchd's wrapper alive until the independently launched app exits.
        let app = executable
            .parent()
            .and_then(std::path::Path::parent)
            .and_then(std::path::Path::parent)
            .context(Message::MacosBundleRequired)?;
        let app = xml(app.to_str().context(Message::InstallationPathMustBeUTF8)?);
        format!(
            "<string>/usr/bin/open</string><string>-W</string><string>-n</string><string>-g</string><string>-a</string><string>{app}</string><string>--env</string><string>WLANLOGIN_MANAGED=1</string><string>--args</string><string>--user</string><string>daemon</string><string>--background</string>"
        )
    };
    let log = xml(log.to_str().context(Message::LogPathMustBeUTF8)?);
    let domain = format!("gui/{uid}");
    let target = format!("{domain}/{LABEL}");
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LABEL}</string>
<key>AssociatedBundleIdentifiers</key><array><string>{app_id}</string></array>
<key>ProgramArguments</key><array>{arguments}</array>
<key>EnvironmentVariables</key><dict><key>WLANLOGIN_MANAGED</key><string>1</string></dict>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
<key>ThrottleInterval</key><integer>30</integer>
<key>StandardErrorPath</key><string>{log}</string>
</dict></plist>
"#,
        app_id = crate::infra::identity::APP_ID
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
pub(crate) fn legacy_plan(paths: &Paths) -> Result<Plan> {
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

pub(crate) fn bundle_definition(executable: &std::path::Path, system: bool) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app")
        .then(|| app.join(crate::infra::identity::macos_service_path(system)))
}

pub(crate) fn app_service_plan(executable: PathBuf, system: bool, uid: u32) -> Result<Plan> {
    let definition =
        bundle_definition(&executable, system).context(Message::MacosBundleRequired)?;
    let domain = if system {
        "system".into()
    } else {
        format!("gui/{uid}")
    };
    let target = format!("{domain}/{LABEL}");
    Ok(Plan {
        scope: if system {
            crate::infra::scope::Scope::System
        } else {
            crate::infra::scope::Scope::User
        },
        manager: Manager::MacosAppService {
            domain: domain.clone(),
            target: target.clone(),
            executable: executable.clone(),
        },
        definition,
        content: crate::infra::identity::macos_service_plist(system),
        stop: vec![Invocation::new("launchctl", ["bootout", &target]).optional()],
        start: vec![super::app_service::command(
            &executable,
            &domain,
            "register",
        )],
        remove: vec![super::app_service::command(
            &executable,
            &domain,
            "unregister",
        )],
        reload: vec![],
        executable,
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn native_plan(paths: &Paths) -> Result<Plan> {
    let system = paths.scope == crate::infra::scope::Scope::System;
    app_service_plan(
        crate::infra::paths::macos_bundle(paths.scope)?
            .join("Contents/MacOS")
            .join(crate::infra::identity::APP_NAME),
        system,
        unsafe { libc::geteuid() },
    )
}

#[cfg(target_os = "macos")]
pub(crate) fn plan(paths: &Paths) -> Result<Plan> {
    let native = native_plan(paths)?;
    let legacy = legacy_plan(paths)?;
    // Preserve control and uninstallation of installations made by old binaries.
    if !native.definition.try_exists()? && legacy.definition.try_exists()? {
        return Ok(legacy);
    }
    Ok(native)
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
    let bundled = match paths.scope {
        Scope::User => crate::infra::paths::macos_bundle(Scope::System)?
            .join(crate::infra::identity::macos_service_path(true)),
        Scope::System => {
            let Some(home) = original_user_home()? else {
                return Ok(None);
            };
            crate::infra::paths::app_dir(home.join("Library/Application Support"))
                .join(crate::infra::identity::APP_BUNDLE)
                .join(crate::infra::identity::macos_service_path(false))
        }
    };
    Ok(Some(if bundled.try_exists()? {
        bundled
    } else {
        other
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_services_run_signed_bundle_programs_without_a_launcher() -> Result<()> {
        let exe =
            PathBuf::from("/Users/Test User/Applications/wlanlogin.app/Contents/MacOS/wlanlogin");
        for system in [false, true] {
            let plan = app_service_plan(exe.clone(), system, 501)?;
            assert!(matches!(plan.manager, Manager::MacosAppService { .. }));
            assert!(plan.content.contains("<key>BundleProgram</key>"));
            assert!(!plan.content.contains("/usr/bin/open"));
            assert!(!plan.content.contains("/Users/Test User"));
            assert_eq!(
                plan.definition,
                bundle_definition(&exe, system).context(Message::MacosBundleRequired)?
            );
            assert!(
                plan.start
                    .iter()
                    .all(|c| c.program == exe && c.args.last().is_some_and(|a| a == "register"))
            );
            assert!(
                plan.remove
                    .iter()
                    .all(|c| c.program == exe && c.args.last().is_some_and(|a| a == "unregister"))
            );
        }
        assert!(app_service_plan("/tmp/loose-binary".into(), false, 501).is_err());
        Ok(())
    }
    #[test]
    fn user_service_uses_launch_services_and_root_service_runs_directly() -> Result<()> {
        let exe = PathBuf::from("/Applications/Test & Tool.app/Contents/MacOS/wlanlogin");
        let user = macos_plan(exe.clone(), "/tmp/job".into(), "/tmp/log".into(), 501)?;
        assert!(
            user.content
                .contains("<string>/usr/bin/open</string><string>-W</string>")
        );
        assert!(user.content.contains("Test &amp; Tool.app</string>"));
        assert!(user.content.contains(
            "<string>--user</string><string>daemon</string><string>--background</string>"
        ));
        assert!(user.content.contains("WLANLOGIN_MANAGED=1"));
        let root = macos_system_plan(exe, "/tmp/job".into(), "/tmp/log".into())?;
        assert!(!root.content.contains("/usr/bin/open"));
        assert!(root.content.contains(
            "<string>--system</string><string>daemon</string><string>--background</string>"
        ));
        assert!(root.content.contains("<key>WLANLOGIN_MANAGED</key>"));
        for plan in [&user, &root] {
            assert!(plan.content.contains("<key>AssociatedBundleIdentifiers</key><array><string>dev.cxcs.tools.wlanlogin</string></array>"));
        }
        Ok(())
    }
}
