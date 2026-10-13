//! Default registered-service restart workflow.
use crate::infra::{error::Result, paths::Paths};
use crate::system::define::services::Action;

pub(crate) async fn run(paths: &Paths) -> Result<()> {
    // Delegate the operation to the service manager after shared validation.
    crate::system::shared::flow::control::manage(paths, Action::Restart).await
}
