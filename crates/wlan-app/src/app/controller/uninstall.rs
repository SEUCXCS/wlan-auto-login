//! Uninstall policy, including optional removal of saved credentials.
use crate::infra::{error::Result, paths::Paths};
use crate::system::{credentials, shared::services};
pub async fn run(paths: &Paths, purge: bool) -> Result<()> {
    services::uninstall(paths, purge, credentials::delete(paths)).await
}
