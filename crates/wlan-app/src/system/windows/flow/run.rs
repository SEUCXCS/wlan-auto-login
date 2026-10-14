//! SCM selection occurs in app/bootstrap before Tokio starts.
use crate::infra::{error::Result, paths::Paths};

pub(crate) async fn run(
    paths: &Paths,
    managed: bool,
    operation: impl std::future::Future<Output = Result<()>>,
) -> Result<()> {
    // User task daemons must release their inherited console.
    if managed {
        crate::system::windows::runtime::service::detach_console();
    }

    crate::system::shared::flow::run::run(paths, managed, operation).await
}
