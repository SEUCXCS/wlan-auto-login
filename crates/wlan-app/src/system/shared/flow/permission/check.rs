use crate::infra::{error::Result, scope::Scope};
use crate::system::define::permission::{PermissionProvider, PermissionReport};

pub(crate) async fn run(
    provider: &impl PermissionProvider,
    scope: Scope,
    id: Option<&str>,
) -> Result<PermissionReport> {
    // Status checks never request authorization.
    super::report::report(provider, scope, id, false, false).await
}
