//! Linux supplies the journal/file reader to the common complete log workflow.
use crate::infra::{error::Result, paths::Paths};
use crate::system::define::services::LogOptions;

pub(crate) async fn run(
    paths: &Paths,
    options: LogOptions,
    output: &mut impl std::io::Write,
    stop: impl std::future::Future<Output = std::io::Result<()>>,
) -> Result<()> {
    // Journal execution stays inside the native adapter, including cancellation.
    crate::system::shared::flow::logs::run_with(
        paths,
        options,
        output,
        stop,
        &crate::system::linux::services::logs::Backend,
    )
    .await
}
