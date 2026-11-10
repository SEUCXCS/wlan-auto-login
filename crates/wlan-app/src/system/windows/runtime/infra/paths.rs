use crate::infra::error::Message;
use crate::infra::error::{Context, Result};

use crate::infra::identity::DAEMON_BINARY;
use crate::infra::identity::{APP_NAME, ORGANIZATION};
use directories::ProjectDirs;
use std::path::{Path, PathBuf};

/// Append only the application's two directory components to a platform base.
pub fn app_dir(base: impl AsRef<Path>) -> PathBuf {
    base.as_ref().join(ORGANIZATION).join(APP_NAME)
}

pub fn project_dirs() -> Option<ProjectDirs> {
    // `from` ignores the organization on Linux and joins it with dots on macOS.
    ProjectDirs::from_path(app_dir(""))
}

pub fn user_daemon_executable() -> Result<PathBuf> {
    Ok(project_dirs()
        .context(Message::FailedToDetermineInstallationDirectory)?
        .data_local_dir()
        .join("bin")
        .join(DAEMON_BINARY))
}

pub fn system_daemon_executable() -> Result<PathBuf> {
    {
        Ok(app_dir(known_folder(
            &windows_sys::Win32::UI::Shell::FOLDERID_ProgramFiles,
        )?)
        .join(DAEMON_BINARY))
    }
}

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
            {
                let base = app_dir(known_folder(
                    &windows_sys::Win32::UI::Shell::FOLDERID_ProgramData,
                )?);
                return Ok(Self {
                    scope,
                    config: base.join("state"),
                    runtime: base.join("run"),
                    secrets: base.join("secrets"),
                });
            }
        }
        let dirs = project_dirs().context(Message::FailedToDetermineUserConfigurationDirectory)?;
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

pub fn known_folder(id: &windows_sys::core::GUID) -> Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;

    struct PathBuffer(*mut u16);
    impl Drop for PathBuffer {
        fn drop(&mut self) {
            // SAFETY: SHGetKnownFolderPath's output must be freed on success
            // and failure. CoTaskMemFree also accepts a null pointer.
            unsafe { windows_sys::Win32::System::Com::CoTaskMemFree(self.0.cast()) };
        }
    }

    // SAFETY: the shell initializes its output; the guard owns that allocation
    // before checking the result and retains it throughout UTF-16 conversion.
    unsafe {
        let mut value = std::ptr::null_mut();
        let result = windows_sys::Win32::UI::Shell::SHGetKnownFolderPath(
            id,
            0,
            std::ptr::null_mut(),
            &mut value,
        );
        let value = PathBuffer(value);
        crate::infra::error::ensure!(
            result >= 0,
            Message::FailedToResolveSystemDirectoryResultX {
                result: format!("{:#x}", result)
            }
        );
        let mut length = 0;
        while *value.0.add(length) != 0 {
            length += 1;
        }
        let path = std::ffi::OsString::from_wide(std::slice::from_raw_parts(value.0, length));
        Ok(path.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::scope::Scope;

    #[test]
    fn user_directories_preserve_both_components_on_every_platform() -> Result<()> {
        let base = directories::BaseDirs::new().context("base directories unavailable")?;
        let user = Paths::new(Scope::User)?;
        let config = base.config_dir().join("SEUCXCS").join("wlanlogin");
        let data = base.data_local_dir().join("SEUCXCS").join("wlanlogin");

        let (config, data) = (config.join("config"), data.join("data"));
        assert_eq!(user.config, config);
        assert_eq!(user.runtime, config);
        assert_eq!(user.secrets, config);

        assert_eq!(
            user_daemon_executable()?,
            data.join("bin").join(DAEMON_BINARY)
        );

        let system = Paths::new(Scope::System)?;
        for path in [&system.config, &system.runtime, &system.secrets] {
            assert_ne!(path, &user.config);
        }
        assert_ne!(system_daemon_executable()?, user_daemon_executable()?);
        Ok(())
    }
}
