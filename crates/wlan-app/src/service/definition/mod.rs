//! Platform service definitions, installation paths and ownership checks.
#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(any(target_os = "linux", test))]
pub(super) use linux::{linux_plan, linux_system_plan};
#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(any(target_os = "macos", test))]
pub(super) use macos::{macos_plan, macos_system_plan};
#[cfg(any(windows, test))]
mod windows;
#[cfg(any(windows, test))]
pub(super) use windows::{windows_plan, windows_system_plan};

use super::{command::Invocation, manager::Manager};
use crate::config::Paths;
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) const LABEL: &str = "org.seucxcs.cxxy-wlanlogin";

pub(super) struct Plan {
    pub(super) scope: crate::scope::Scope,
    pub(super) manager: Manager,
    pub(super) executable: PathBuf,
    pub(super) definition: PathBuf,
    pub(super) content: String,
    pub(super) stop: Vec<Invocation>,
    pub(super) start: Vec<Invocation>,
    pub(super) remove: Vec<Invocation>,
    pub(super) reload: Vec<Invocation>,
}

pub(super) fn plan(paths: &Paths) -> Result<Plan> {
    if paths.scope == crate::scope::Scope::System {
        return system_plan(paths);
    }
    let dirs = directories::ProjectDirs::from("org", "seucxcs", "cxxy-wlanlogin")
        .context("无法确定安装目录")?;
    let executable = dirs.data_local_dir().join("bin").join(if cfg!(windows) {
        "cxxy-wlanlogin-daemon.exe"
    } else {
        "cxxy-wlanlogin-daemon"
    });
    #[cfg(target_os = "linux")]
    {
        let base = directories::BaseDirs::new().context("无法确定用户服务目录")?;
        let _ = paths;
        linux_plan(
            executable,
            base.config_dir()
                .join("systemd/user/cxxy-wlanlogin.service"),
        )
    }
    #[cfg(target_os = "macos")]
    {
        let base = directories::BaseDirs::new().context("无法确定用户服务目录")?;
        let uid = unsafe { libc::geteuid() };
        let log = paths.config.join("daemon.log");
        macos_plan(
            executable,
            base.home_dir()
                .join("Library/LaunchAgents")
                .join(format!("{LABEL}.plist")),
            log,
            uid,
        )
    }
    #[cfg(target_os = "windows")]
    {
        let sid = crate::scope::current_sid()?;
        windows_plan(executable, paths.config.join("daemon-task.xml"), &sid)
    }
}

fn system_plan(paths: &Paths) -> Result<Plan> {
    #[cfg(target_os = "linux")]
    {
        let _ = paths;
        linux_system_plan(
            "/usr/local/libexec/cxxy-wlanlogin/cxxy-wlanlogin-daemon".into(),
            "/etc/systemd/system/cxxy-wlanlogin.service".into(),
        )
    }
    #[cfg(target_os = "macos")]
    {
        macos_system_plan(
            "/Library/Application Support/cxxy-wlanlogin/bin/cxxy-wlanlogin-daemon".into(),
            "/Library/LaunchDaemons/org.seucxcs.cxxy-wlanlogin.plist".into(),
            paths.config.join("daemon.log"),
        )
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::{FOLDERID_ProgramData, FOLDERID_ProgramFiles};
        let _ = paths;
        windows_system_plan(
            crate::config::known_folder(&FOLDERID_ProgramFiles)?
                .join("cxxy-wlanlogin/cxxy-wlanlogin-daemon.exe"),
            crate::config::known_folder(&FOLDERID_ProgramData)?.join("cxxy-wlanlogin/service.json"),
        )
    }
}

pub(crate) fn ensure_no_conflicting_install(paths: &Paths) -> Result<()> {
    use crate::scope::Scope;
    let other = match paths.scope {
        Scope::User => system_plan(&Paths::new(Scope::System)?)?.definition,
        Scope::System => {
            #[cfg(unix)]
            {
                if let Some(home) = original_user_home()? {
                    #[cfg(target_os = "linux")]
                    {
                        home.join(".config/systemd/user/cxxy-wlanlogin.service")
                    }
                    #[cfg(target_os = "macos")]
                    {
                        home.join("Library/LaunchAgents/org.seucxcs.cxxy-wlanlogin.plist")
                    }
                } else {
                    return Ok(());
                }
            }
            #[cfg(windows)]
            {
                plan(&Paths::new(Scope::User)?)?.definition
            }
        }
    };
    if other.try_exists()? {
        assert_owned(&other)?;
        bail!(
            "检测到另一安装范围的服务：{}；请使用该服务对应的范围和权限执行命令；如需切换安装范围，请先卸载该服务（凭据会保留）",
            other.display()
        );
    }
    Ok(())
}

#[cfg(unix)]
fn original_user_home() -> Result<Option<PathBuf>> {
    let Some(uid) = std::env::var("SUDO_UID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|uid| *uid != 0)
    else {
        return Ok(None);
    };
    // SUDO_UID is only a lookup hint for conflict detection, never authority for an operation.
    let mut buffer = vec![0u8; 64 * 1024];
    let mut record: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut();
    // SAFETY: getpwuid_r writes pointers into our live buffer; copy the home path before returning.
    unsafe {
        let code = libc::getpwuid_r(
            uid,
            &mut record,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        );
        anyhow::ensure!(
            code == 0 && !result.is_null(),
            "无法确认 sudo 原用户的已有安装"
        );
        use std::os::unix::ffi::OsStrExt;
        Ok(Some(
            std::ffi::OsStr::from_bytes(std::ffi::CStr::from_ptr(record.pw_dir).to_bytes()).into(),
        ))
    }
}

pub(super) fn assert_owned(path: &Path) -> Result<()> {
    match fs::read_to_string(path) {
        Ok(content) if !content.contains(LABEL) => {
            bail!("拒绝覆盖或删除非本工具创建的服务文件：{}", path.display())
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
#[cfg(any(target_os = "macos", target_os = "windows", test))]
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
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

    pub(super) fn any(&self) -> bool {
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

#[cfg(test)]
mod tests;
