use crate::infra::{
    config::{Config, ConfigKey},
    control,
    error::Result,
    paths::Paths,
};

pub(crate) async fn run(paths: &Paths, key: ConfigKey) -> Result<()> {
    paths.scope.require_access()?;
    let _operation = control::operation_lock(paths).await?;
    let mut config = Config::load(paths)?;
    if config.remove(key) {
        config.save(paths)?;
    }
    Ok(())
}
