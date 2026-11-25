use crate::infra::{
    config::{Config, ConfigKey},
    error::Result,
    paths::Paths,
};

pub(crate) fn run(paths: &Paths, key: ConfigKey) -> Result<String> {
    Ok(Config::load(paths)?.get(key)?.to_owned())
}
