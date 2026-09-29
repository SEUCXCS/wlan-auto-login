use crate::infra::command::Invocation;
use crate::infra::error::Message;
use crate::infra::error::{Context, Result, bail};
#[cfg(target_os = "linux")]
use crate::infra::paths::Paths;
use crate::system::define::services::{LABEL, Manager, Plan};
#[cfg(target_os = "linux")]
use crate::system::shared::services::artifacts::original_user_home;
use std::path::{Path, PathBuf};

pub(crate) fn linux_system_plan(executable: PathBuf, definition: PathBuf) -> Result<Plan> {
    let quoted = systemd_quote(&executable)?;
    let reload = || Invocation::new("systemctl", ["--system", "daemon-reload"]);
    Ok(Plan {
        scope: crate::infra::scope::Scope::System,
        manager: Manager::Systemd { system: true },
        content: format!(
            "# {LABEL}\n[Unit]\nDescription=Student_CX automatic login\nAfter=NetworkManager.service\nWants=NetworkManager.service\nStartLimitIntervalSec=0\n\n[Service]\nType=simple\nExecStart={quoted} --system daemon --background\n# ApplicationLog=daemon.log\nRestart=on-failure\nRestartSec=30\nTimeoutStopSec=25\nStateDirectory=cxxy-wlanlogin\nStateDirectoryMode=0700\nRuntimeDirectory=cxxy-wlanlogin\nRuntimeDirectoryMode=0700\nUMask=0077\nNoNewPrivileges=true\nPrivateTmp=true\nProtectHome=true\nProtectSystem=strict\n# CREDENTIALS\n\n[Install]\nWantedBy=multi-user.target\n"
        ),
        executable,
        definition,
        stop: vec![
            Invocation::new("systemctl", ["--system", "stop", "cxxy-wlanlogin.service"]).optional(),
        ],
        start: vec![
            reload(),
            Invocation::new(
                "systemctl",
                ["--system", "enable", "--now", "cxxy-wlanlogin.service"],
            ),
            Invocation::new(
                "systemctl",
                ["--system", "is-active", "--quiet", "cxxy-wlanlogin.service"],
            ),
        ],
        remove: vec![Invocation::new(
            "systemctl",
            ["--system", "disable", "--now", "cxxy-wlanlogin.service"],
        )],
        reload: vec![reload()],
    })
}

pub(crate) fn linux_plan(executable: PathBuf, definition: PathBuf) -> Result<Plan> {
    let quoted = systemd_quote(&executable)?;
    let content = format!(
        "# {LABEL}\n[Unit]\nDescription=Student_CX automatic login\nStartLimitIntervalSec=0\n\n[Service]\nType=simple\nExecStart={quoted} daemon --background\n# ApplicationLog=daemon.log\nRestart=on-failure\nRestartSec=30\nTimeoutStopSec=25\n\n[Install]\nWantedBy=default.target\n"
    );
    let reload = || Invocation::new("systemctl", ["--user", "daemon-reload"]);
    Ok(Plan {
        scope: crate::infra::scope::Scope::User,
        manager: Manager::Systemd { system: false },
        executable,
        definition,
        content,
        stop: vec![
            Invocation::new("systemctl", ["--user", "stop", "cxxy-wlanlogin.service"]).optional(),
        ],
        start: vec![
            reload(),
            Invocation::new(
                "systemctl",
                ["--user", "enable", "--now", "cxxy-wlanlogin.service"],
            ),
            Invocation::new(
                "systemctl",
                ["--user", "is-active", "--quiet", "cxxy-wlanlogin.service"],
            ),
        ],
        remove: vec![Invocation::new(
            "systemctl",
            ["--user", "disable", "--now", "cxxy-wlanlogin.service"],
        )],
        reload: vec![reload()],
    })
}
pub(crate) fn systemd_quote(path: &Path) -> Result<String> {
    let text = path.to_str().context(Message::SystemdPathMustBeUTF8)?;
    if text.chars().any(char::is_control) {
        bail!(Message::InstallationPathContainsControlCharacters);
    }
    Ok(format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('$', "$$")
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn plan(paths: &Paths) -> Result<Plan> {
    if paths.scope == crate::infra::scope::Scope::System {
        return system_plan(paths);
    }
    let dirs = directories::ProjectDirs::from("org", "seucxcs", "cxxy-wlanlogin")
        .context(Message::FailedToDetermineInstallationDirectory)?;
    let executable = dirs.data_local_dir().join("bin").join(if cfg!(windows) {
        "cxxy-wlanlogin-daemon.exe"
    } else {
        "cxxy-wlanlogin-daemon"
    });

    {
        let base =
            directories::BaseDirs::new().context(Message::FailedToDetermineUserServiceDirectory)?;
        let _ = paths;
        linux_plan(
            executable,
            base.config_dir()
                .join("systemd/user/cxxy-wlanlogin.service"),
        )
    }
}
#[cfg(target_os = "linux")]
fn system_plan(paths: &Paths) -> Result<Plan> {
    {
        let _ = paths;
        linux_system_plan(
            "/usr/local/libexec/cxxy-wlanlogin/cxxy-wlanlogin-daemon".into(),
            "/etc/systemd/system/cxxy-wlanlogin.service".into(),
        )
    }
}
#[cfg(target_os = "linux")]
pub(crate) fn conflicting_definition(paths: &Paths) -> Result<Option<PathBuf>> {
    use crate::infra::scope::Scope;
    let other = match paths.scope {
        Scope::User => system_plan(&Paths::new(Scope::System)?)?.definition,
        Scope::System => {
            if let Some(home) = original_user_home()? {
                home.join(".config/systemd/user/cxxy-wlanlogin.service")
            } else {
                return Ok(None);
            }
        }
    };
    Ok(Some(other))
}
