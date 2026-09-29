//! Platform service definitions, installation paths and ownership checks.
use crate::infra::error::Message;
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
use crate::infra::error::{Context, Result, bail};
use crate::infra::paths::Paths;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) const LABEL: &str = "org.seucxcs.cxxy-wlanlogin";

pub(super) struct Plan {
    pub(super) scope: crate::infra::scope::Scope,
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
    #[cfg(target_os = "linux")]
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
    #[cfg(target_os = "macos")]
    {
        let base =
            directories::BaseDirs::new().context(Message::FailedToDetermineUserServiceDirectory)?;
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
        let sid = crate::infra::scope::current_sid()?;
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
            crate::infra::paths::known_folder(&FOLDERID_ProgramFiles)?
                .join("cxxy-wlanlogin/cxxy-wlanlogin-daemon.exe"),
            crate::infra::paths::known_folder(&FOLDERID_ProgramData)?
                .join("cxxy-wlanlogin/service.json"),
        )
    }
}

pub(crate) fn ensure_no_conflicting_install(paths: &Paths) -> Result<()> {
    use crate::infra::scope::Scope;
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
        bail!(Message::AServiceIsInstalledInAnotherScope {
            value0: (other.display()).to_string()
        });
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
        crate::infra::error::ensure!(
            code == 0 && !result.is_null(),
            Message::FailedToResolveTheOriginalSudoUser
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
            bail!(Message::RefusingToReplaceOrRemoveAService {
                value0: (path.display()).to_string()
            })
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
        Err(error) => Err(error).with_context(|| {
            (Message::FailedToInspectInstallationFile {
                value0: (path.display()).to_string(),
            })
            .to_string()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::linux::systemd_quote;
    use super::*;

    #[test]
    fn system_services_boot_without_a_user_session() -> Result<()> {
        let exe = PathBuf::from("/opt/Test App/daemon");
        let definition = PathBuf::from("/tmp/system-definition");
        let linux = linux_system_plan(exe.clone(), definition.clone())?;
        assert!(linux.content.contains("--system daemon"));
        assert!(linux.content.contains("daemon --background"));
        assert!(linux.content.contains("# ApplicationLog=daemon.log"));
        assert!(linux.content.contains("WantedBy=multi-user.target"));
        assert!(!linux.content.contains("--user"));
        let macos = macos_system_plan(exe.clone(), definition.clone(), "/tmp/log".into())?;
        assert!(macos.content.contains("<string>--system</string>"));
        assert!(
            macos
                .start
                .iter()
                .any(|cmd| cmd.args.iter().any(|a| a == "system"))
        );
        assert!(!macos.start.iter().any(|cmd| {
            cmd.args
                .iter()
                .any(|a| a.to_string_lossy().contains("gui/"))
        }));
        let windows = windows_system_plan(exe, definition)?;
        assert!(windows.start.iter().any(|cmd| {
            cmd.args
                .iter()
                .any(|a| a.to_string_lossy().contains("--system daemon --service"))
        }));
        assert!(
            windows
                .start
                .iter()
                .any(|cmd| cmd.args.iter().any(|a| a == "auto"))
        );
        assert!(matches!(windows.manager, Manager::WindowsService));
        for plan in [linux, macos, windows] {
            assert_eq!(plan.scope, crate::infra::scope::Scope::System);
            assert!(plan.content.contains(LABEL));
            assert!(!plan.content.contains("user_password"));
        }
        Ok(())
    }
    #[test]
    fn generated_services_use_user_sessions_without_credentials() -> Result<()> {
        let executable = PathBuf::from("/home/Test User/bin/cxxy");
        let definition = PathBuf::from("/tmp/definition");
        let plans = [
            linux_plan(executable.clone(), definition.clone())?,
            macos_plan(
                executable.clone(),
                definition.clone(),
                "/tmp/daemon.log".into(),
                501,
            )?,
            windows_plan(executable, definition, "S-1-5-21-123")?,
        ];
        for plan in &plans {
            assert!(plan.content.contains("daemon"));
            assert!(!plan.content.contains("--password"));
            assert!(!plan.content.contains("CXXY_WLAN_PASSWORD"));
            assert!(plan.content.contains(LABEL));
        }
        assert!(plans[0].content.contains("WantedBy=default.target"));
        assert!(plans[0].content.contains("daemon --background"));
        assert!(plans[1].content.contains("<key>RunAtLoad</key><true/>"));
        assert!(
            plans[2]
                .content
                .contains("<LogonType>InteractiveToken</LogonType>")
        );
        assert!(
            plans[2]
                .content
                .contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>")
        );
        Ok(())
    }
    #[test]
    fn service_paths_are_escaped_and_unrelated_files_are_preserved() -> Result<()> {
        assert_eq!(
            systemd_quote(Path::new("/tmp/a %$\"b"))?,
            "\"/tmp/a %%$$\\\"b\""
        );
        assert!(systemd_quote(Path::new("/tmp/a\nb")).is_err());
        assert_eq!(xml("a<&\""), "a&lt;&amp;&quot;");
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("service");
        fs::write(&file, "a different service")?;
        assert!(assert_owned(&file).is_err());
        assert_eq!(fs::read_to_string(file)?, "a different service");
        Ok(())
    }
}
