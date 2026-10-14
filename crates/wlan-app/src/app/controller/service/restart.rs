//! Request the normalized restart workflow for the selected platform.
use crate::infra::{error::Result, paths::Paths};

pub async fn run(paths: &Paths) -> Result<()> {
    crate::system::flow::restart::run(paths).await
}
