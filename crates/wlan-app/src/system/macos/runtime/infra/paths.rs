use crate::infra::error::Message;
use crate::infra::error::{Context, Result};

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

pub fn macos_bundle(scope: crate::infra::scope::Scope) -> Result<PathBuf> {
    let root = match scope {
        crate::infra::scope::Scope::User => project_dirs()
            .context(Message::FailedToDetermineInstallationDirectory)?
            .data_local_dir()
            .to_owned(),
        crate::infra::scope::Scope::System => app_dir("/Library/Application Support"),
    };
    Ok(root.join(crate::infra::identity::APP_BUNDLE))
}

pub fn user_daemon_executable() -> Result<PathBuf> {
    return Ok(macos_bundle(crate::infra::scope::Scope::User)?
        .join("Contents/MacOS")
        .join(APP_NAME));
}

pub fn system_daemon_executable() -> Result<PathBuf> {
    {
        Ok(macos_bundle(crate::infra::scope::Scope::System)?
            .join("Contents/MacOS")
            .join(APP_NAME))
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
            return Ok(Self {
                scope,
                config: app_dir("/Library/Application Support").join("state"),
                runtime: app_dir("/var/run"),
                secrets: app_dir("/Library/Application Support").join("secrets"),
            });
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

        assert_eq!(user.config, config);
        assert_eq!(user.runtime, config);
        assert_eq!(user.secrets, config);

        assert_eq!(
            user_daemon_executable()?,
            data.join("wlanlogin.app/Contents/MacOS/wlanlogin")
        );

        let system = Paths::new(Scope::System)?;
        for path in [&system.config, &system.runtime, &system.secrets] {
            assert_ne!(path, &user.config);
        }
        assert_ne!(system_daemon_executable()?, user_daemon_executable()?);
        Ok(())
    }
}
