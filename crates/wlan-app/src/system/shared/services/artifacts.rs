//! Installation artifacts, ownership validation and native plan selection.
use crate::infra::{
    error::{Context, Message, Result, bail},
    paths::Paths,
};
use crate::system::define::services::LABEL;
pub(crate) use crate::system::define::services::Plan;
#[cfg(test)]
pub(crate) use crate::system::linux::services::definition::{linux_plan, linux_system_plan};
#[cfg(test)]
pub(crate) use crate::system::macos::services::definition::app_service_plan;
pub(crate) use crate::system::native::services::definition::plan;
#[cfg(test)]
pub(crate) use crate::system::windows::services::definition::{windows_plan, windows_system_plan};
#[cfg(any(unix, test))]
use std::path::PathBuf;
use std::{fs, path::Path};
pub(crate) fn ensure_no_conflicting_install(paths: &Paths) -> Result<()> {
    if let Some(other) = crate::system::native::services::definition::conflicting_definition(paths)?
        && other.try_exists()?
    {
        assert_owned(&other)?;
        bail!(Message::AServiceIsInstalledInAnotherScope {
            value0: other.display().to_string()
        });
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn original_user_home() -> Result<Option<PathBuf>> {
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

pub(crate) fn assert_owned(path: &Path) -> Result<()> {
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
#[cfg(any(target_os = "windows", test))]
pub(crate) fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub(crate) use crate::system::define::services::Artifacts;

impl Artifacts {
    pub fn read(plan: &Plan) -> Result<Self> {
        assert_owned(&plan.definition)?;
        let executable = is_file(&plan.executable)?;
        #[cfg(target_os = "macos")]
        let executable = if let Some(app) =
            crate::system::macos::services::bundle::enclosing(&plan.executable)
        {
            executable
                && is_file(&app.join("Contents/Info.plist"))?
                && is_file(&app.join("Contents/_CodeSignature/CodeResources"))?
        } else {
            executable
        };
        Ok(Self {
            definition: is_file(&plan.definition)?,
            executable,
        })
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
    use super::*;
    use crate::system::define::services::Manager;
    use crate::system::linux::services::definition::systemd_quote;

    #[test]
    fn system_services_boot_without_a_user_session() -> Result<()> {
        let exe = PathBuf::from("/opt/Test App/daemon");
        let definition = PathBuf::from("/tmp/system-definition");
        let linux = linux_system_plan(exe.clone(), definition.clone())?;
        assert!(linux.content.contains("--system service run"));
        assert!(linux.content.contains("service run"));
        assert!(
            linux
                .content
                .contains("Description=CXCS WLAN Login — Student_CX automatic login\n")
        );
        assert!(linux.content.contains("WantedBy=multi-user.target"));
        assert!(linux.content.contains("StateDirectory=SEUCXCS/wlanlogin\n"));
        assert!(
            linux
                .content
                .contains("RuntimeDirectory=SEUCXCS/wlanlogin\n")
        );
        assert!(!linux.content.contains("--user"));
        let macos = app_service_plan(
            "/Applications/wlanlogin.app/Contents/MacOS/wlanlogin".into(),
            true,
            0,
        )?;
        assert!(macos.content.contains("<string>--system</string>"));
        assert!(
            macos
                .start
                .iter()
                .any(|cmd| cmd.args.iter().any(|a| a == "--system"))
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
                .any(|a| a.to_string_lossy().contains("--system service run"))
        }));
        assert!(
            windows
                .start
                .iter()
                .any(|cmd| cmd.args.iter().any(|a| a == "auto"))
        );
        assert!(matches!(windows.manager, Manager::WindowsService));
        for command in &windows.start[..2] {
            assert!(
                command
                    .args
                    .windows(2)
                    .any(|args| { args[0] == "DisplayName=" && args[1] == "CXCS WLAN Login" })
            );
            assert_eq!(command.args[1], LABEL);
        }
        for plan in [linux, macos, windows] {
            assert_eq!(plan.scope, crate::infra::scope::Scope::System);
            assert!(plan.content.contains(LABEL));
            assert!(!plan.content.contains("user_password"));
        }
        Ok(())
    }
    #[test]
    fn generated_services_use_user_sessions_without_credentials() -> Result<()> {
        let executable = PathBuf::from("/home/Test User/bin/wlanlogin");
        let definition = PathBuf::from("/tmp/definition");
        let plans = [
            linux_plan(executable.clone(), definition.clone())?,
            app_service_plan(
                "/Applications/wlanlogin.app/Contents/MacOS/wlanlogin".into(),
                false,
                501,
            )?,
            windows_plan(executable, definition, "S-1-5-21-123")?,
        ];
        for plan in &plans {
            assert!(
                plan.content.contains("service run")
                    || plan
                        .content
                        .contains("<string>service</string><string>run</string>")
            );
            assert!(!plan.content.contains("--password"));
            assert!(!plan.content.contains("WLANLOGIN_PASSWORD"));
            assert!(plan.content.contains(LABEL));
        }
        assert!(plans[0].content.contains("WantedBy=default.target"));
        assert!(
            plans[0]
                .content
                .contains("Description=CXCS WLAN Login — Student_CX automatic login\n")
        );
        assert!(
            plans[2].content.contains(
                "<Description>CXCS WLAN Login — Student_CX automatic login</Description>"
            )
        );
        assert!(plans[0].content.contains("service run"));
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
