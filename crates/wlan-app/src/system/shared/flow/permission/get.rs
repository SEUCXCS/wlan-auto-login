use crate::infra::{error::Result, scope::Scope};
use crate::system::define::permission::{PermissionProvider, PermissionReport};

pub(crate) async fn run(
    provider: &impl PermissionProvider,
    scope: Scope,
    id: &str,
    interactive: bool,
) -> Result<PermissionReport> {
    // Authorization needs explicit foreground interaction.
    super::report::report(provider, scope, Some(id), true, interactive).await
}
