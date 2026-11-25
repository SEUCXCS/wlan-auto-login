use crate::infra::{
    config::{Config, ConfigKey},
    error::Result,
    paths::Paths,
};

pub(crate) fn run(paths: &Paths) -> Result<Vec<(ConfigKey, String)>> {
    let config = Config::load(paths)?;
    ConfigKey::ALL
        .into_iter()
        .map(|key| Ok((key, config.get(key)?.to_owned())))
        .collect()
}
