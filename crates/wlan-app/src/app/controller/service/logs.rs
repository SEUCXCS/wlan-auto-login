//! Delegate log execution to the selected system workflow.
use crate::infra::{error::Result, paths::Paths};
pub(crate) use crate::system::define::services::LogOptions;

pub async fn run(
    paths: &Paths,
    options: LogOptions,
    output: &mut impl std::io::Write,
    stop: impl std::future::Future<Output = std::io::Result<()>>,
) -> Result<()> {
    crate::system::flow::logs::run(paths, options, output, stop).await
}
