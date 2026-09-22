use super::{Invocation, LABEL, Manager, Plan};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

pub(in crate::service) fn linux_system_plan(
    executable: PathBuf,
    definition: PathBuf,
) -> Result<Plan> {
    let quoted = systemd_quote(&executable)?;
    let reload = || Invocation::new("systemctl", ["--system", "daemon-reload"]);
    Ok(Plan {
        scope: crate::scope::Scope::System,
        manager: Manager::Systemd { system: true },
        content: format!(
            "# {LABEL}\n[Unit]\nDescription=Student_CX automatic login\nAfter=NetworkManager.service\nWants=NetworkManager.service\nStartLimitIntervalSec=0\n\n[Service]\nType=simple\nExecStart={quoted} --system daemon\nRestart=on-failure\nRestartSec=30\nTimeoutStopSec=25\nStateDirectory=cxxy-wlanlogin\nStateDirectoryMode=0700\nRuntimeDirectory=cxxy-wlanlogin\nRuntimeDirectoryMode=0700\nUMask=0077\nNoNewPrivileges=true\nPrivateTmp=true\nProtectHome=true\nProtectSystem=strict\n# CREDENTIALS\n\n[Install]\nWantedBy=multi-user.target\n"
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

pub(in crate::service) fn linux_plan(executable: PathBuf, definition: PathBuf) -> Result<Plan> {
    let quoted = systemd_quote(&executable)?;
    let content = format!(
        "# {LABEL}\n[Unit]\nDescription=Student_CX automatic login\nStartLimitIntervalSec=0\n\n[Service]\nType=simple\nExecStart={quoted} daemon\nRestart=on-failure\nRestartSec=30\nTimeoutStopSec=25\n\n[Install]\nWantedBy=default.target\n"
    );
    let reload = || Invocation::new("systemctl", ["--user", "daemon-reload"]);
    Ok(Plan {
        scope: crate::scope::Scope::User,
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
pub(super) fn systemd_quote(path: &Path) -> Result<String> {
    let text = path.to_str().context("systemd 路径必须是 UTF-8")?;
    if text.chars().any(char::is_control) {
        bail!("安装路径包含控制字符");
    }
    Ok(format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('$', "$$")
    ))
}
