//! The ordinary process entry has no additional native hosting requirements.
use crate::infra::{error::Result, paths::Paths};

pub(crate) async fn run(
    _paths: &Paths,
    _managed: bool,
    operation: impl std::future::Future<Output = Result<()>>,
) -> Result<()> {
    // The application supplies its daemon future, preserving app -> system -> infra.
    operation.await
}
