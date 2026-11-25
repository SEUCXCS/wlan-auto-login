use crate::infra::{
    config::{Config, ConfigKey},
    control,
    error::Result,
    paths::Paths,
};

pub(crate) async fn run(paths: &Paths, key: ConfigKey, value: String) -> Result<()> {
    paths.scope.require_access()?;
    let _operation = control::operation_lock(paths).await?;
    let mut config = Config::load(paths)?;
    config.set(key, value)?;
    config.save(paths)
}
