use crate::infra::{error::Result, paths::Paths};
use crate::system::shared::services::{self, Action};
pub async fn run(paths: &Paths) -> Result<()> {
    services::manage(paths, Action::Start).await
}
