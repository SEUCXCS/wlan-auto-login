use crate::infra::{error::Result, paths::Paths};
use crate::system::define::services::{LogOptions, LogSource, Plan};
pub(crate) fn source(paths: &Paths, _: &Plan, _: LogOptions) -> Result<LogSource> {
    Ok(LogSource::File(paths.config.join("daemon.log")))
}
