//! Linux adds the systemd credential directive; publication/rollback is shared.
use crate::infra::{error::Result, paths::Paths, scope::Scope};

pub(crate) async fn run(paths: &Paths) -> Result<()> {
    // System services receive native encrypted credentials through systemd.
    crate::system::shared::flow::install::run_with(paths, |mut plan| {
        if paths.scope == Scope::System {
            plan.content = plan.content.replace(
                "# CREDENTIALS\n",
                &crate::system::credentials::linux_credential_directive(paths)?,
            );
        }
        Ok(plan)
    })
    .await
}

pub(crate) use crate::system::shared::services::artifacts::ensure_no_conflicting_install as validate_scope;
