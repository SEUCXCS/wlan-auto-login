//! Uninstall policy, including optional removal of saved credentials.
use crate::infra::{error::Result, paths::Paths};
pub async fn run(paths: &Paths, purge: bool) -> Result<()> {
    crate::system::flow::uninstall::run(paths, purge).await
}
