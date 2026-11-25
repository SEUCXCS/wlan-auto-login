use super::{filesystem::write_private, paths::Paths};
use crate::infra::error::Message;
use crate::infra::error::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, fs, str::FromStr};
use wlanlogin_lib::Line;

pub(crate) const CONFIG_FILE: &str = "config.json";

#[derive(Clone, Copy, Default)]
pub(crate) enum LogLevel {
    #[default]
    Info,
    Debug,
}

impl FromStr for LogLevel {
    type Err = crate::infra::error::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "info" => Ok(Self::Info),
            "debug" => Ok(Self::Debug),
            _ => Err(Message::InvalidLogLevel.into()),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ConfigKey {
    LogLevel,
}

impl ConfigKey {
    pub const ALL: [Self; 1] = [Self::LogLevel];

    fn name(self) -> &'static str {
        match self {
            Self::LogLevel => "log.level",
        }
    }

    fn default_value(self) -> &'static str {
        match self {
            Self::LogLevel => "info",
        }
    }

    fn validate(self, value: &str) -> Result<()> {
        match self {
            Self::LogLevel => value.parse::<LogLevel>().map(|_| ()),
        }
    }
}

impl FromStr for ConfigKey {
    type Err = crate::infra::error::Error;

    fn from_str(key: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.name() == key)
            .context(Message::UnknownConfigKey)
    }
}

impl fmt::Display for ConfigKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct Config(BTreeMap<String, String>);

impl Config {
    pub fn load(paths: &Paths) -> Result<Self> {
        let path = paths.config.join(CONFIG_FILE);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error).context(Message::FailedToReadApplicationConfig),
        };
        crate::infra::error::ensure!(metadata.is_file(), Message::InvalidApplicationConfig);
        let bytes = fs::read(path).context(Message::FailedToReadApplicationConfig)?;
        serde_json::from_slice(&bytes).context(Message::InvalidApplicationConfig)
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_private(
            &paths.config.join(CONFIG_FILE),
            &serde_json::to_vec_pretty(self)?,
        )
    }

    pub fn get(&self, key: ConfigKey) -> Result<&str> {
        let value = self.value(key);
        key.validate(value)?;
        Ok(value)
    }

    pub fn set(&mut self, key: ConfigKey, value: String) -> Result<()> {
        key.validate(&value)?;
        self.0.insert(key.name().into(), value);
        Ok(())
    }

    pub fn remove(&mut self, key: ConfigKey) -> bool {
        self.0.remove(key.name()).is_some()
    }

    pub fn log_level(&self) -> Result<LogLevel> {
        self.value(ConfigKey::LogLevel).parse()
    }

    fn value(&self, key: ConfigKey) -> &str {
        self.0
            .get(key.name())
            .map_or(key.default_value(), String::as_str)
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Preferences {
    pub line: Line,
}

impl Preferences {
    pub fn load(paths: &Paths) -> Result<Self> {
        match fs::read(paths.config.join("preferences.json")) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).context(Message::InvalidConnectionPreferences)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).context(Message::FailedToReadConnectionPreferences),
        }
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        write_private(
            &paths.config.join("preferences.json"),
            &serde_json::to_vec_pretty(self)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_level_defaults_and_persists_in_the_selected_scope() -> Result<()> {
        let root = tempfile::tempdir()?;
        let user = Paths::test(root.path().join("user"));
        let mut system = Paths::test(root.path().join("system"));
        system.scope = crate::infra::scope::Scope::System;
        let key = ConfigKey::LogLevel;

        let mut config = Config::load(&user)?;
        assert_eq!(config.get(key)?, "info");
        assert!("log.unknown".parse::<ConfigKey>().is_err());
        assert!(config.set(key, "trace".into()).is_err());
        config.set(key, "debug".into())?;
        config.save(&user)?;
        assert_eq!(Config::load(&user)?.get(key)?, "debug");
        assert_eq!(Config::load(&system)?.get(key)?, "info");
        assert!(matches!(Config::load(&user)?.log_level()?, LogLevel::Debug));

        let mut config = Config::load(&user)?;
        assert!(config.remove(key));
        assert!(!config.remove(key));
        config.save(&user)?;
        assert_eq!(Config::load(&user)?.get(key)?, "info");
        assert!(!user.config.join("preferences.json").exists());
        Ok(())
    }
}
