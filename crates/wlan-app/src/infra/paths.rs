use crate::infra::error::Message;
use crate::infra::error::{Context, Result};
use directories::ProjectDirs;
use std::path::PathBuf;

#[derive(Clone)]
pub struct Paths {
    pub scope: crate::infra::scope::Scope,
    pub config: PathBuf,
    pub runtime: PathBuf,
    pub secrets: PathBuf,
}

impl Paths {
    pub fn new(scope: crate::infra::scope::Scope) -> Result<Self> {
        use crate::infra::scope::Scope;
        if scope == Scope::System {
            #[cfg(target_os = "linux")]
            return Ok(Self {
                scope,
                config: "/var/lib/cxxy-wlanlogin".into(),
                runtime: "/run/cxxy-wlanlogin".into(),
                secrets: "/etc/cxxy-wlanlogin".into(),
            });
            #[cfg(target_os = "macos")]
            return Ok(Self {
                scope,
                config: "/Library/Application Support/cxxy-wlanlogin/state".into(),
                runtime: "/var/run/cxxy-wlanlogin".into(),
                secrets: "/Library/Application Support/cxxy-wlanlogin/secrets".into(),
            });
            #[cfg(windows)]
            {
                let base = known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_ProgramData)?
                    .join("cxxy-wlanlogin");
                return Ok(Self {
                    scope,
                    config: base.join("state"),
                    runtime: base.join("run"),
                    secrets: base.join("secrets"),
                });
            }
        }
        let dirs = ProjectDirs::from("org", "seucxcs", "cxxy-wlanlogin")
            .context(Message::FailedToDetermineUserConfigurationDirectory)?;
        Ok(Self {
            scope,
            config: dirs.config_dir().into(),
            runtime: dirs.config_dir().into(),
            secrets: dirs.config_dir().into(),
        })
    }

    #[cfg(test)]
    pub fn test(config: impl Into<PathBuf>) -> Self {
        let config = config.into();
        Self {
            scope: Default::default(),
            runtime: config.clone(),
            secrets: config.clone(),
            config,
        }
    }
}

#[cfg(windows)]
pub fn known_folder(id: &windows_sys::core::GUID) -> Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    // SAFETY: the shell allocates a NUL-terminated UTF-16 string, released with CoTaskMemFree.
    unsafe {
        let mut value = std::ptr::null_mut();
        let result = windows_sys::Win32::UI::Shell::SHGetKnownFolderPath(
            id,
            0,
            std::ptr::null_mut(),
            &mut value,
        );
        crate::infra::error::ensure!(
            result >= 0,
            Message::FailedToResolveSystemDirectoryResultX {
                result: format!("{:#x}", result)
            }
        );
        let mut length = 0;
        while *value.add(length) != 0 {
            length += 1;
        }
        let path = std::ffi::OsString::from_wide(std::slice::from_raw_parts(value, length));
        windows_sys::Win32::System::Com::CoTaskMemFree(value.cast());
        Ok(path.into())
    }
}
