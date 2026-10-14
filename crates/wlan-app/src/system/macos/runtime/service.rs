//! Control managed processes after removing their launchd job.
use crate::infra::{
    control,
    error::{Message, Result, ensure},
    paths::Paths,
};
use crate::system::define::services::Manager;
#[cfg(test)]
use crate::system::define::services::ServiceManager;

/// Acquire before consulting launchd. After bootout, a delayed managed process
/// must fail this check before creating a host or a daemon endpoint.
pub async fn guard(paths: &Paths) -> Result<std::fs::File> {
    let guard = control::managed_lock(paths)?;
    let plan = crate::system::macos::services::definition::plan(paths)?;
    let state = match &plan.manager {
        Manager::MacosAppService { domain, target, .. } => {
            crate::system::macos::services::manager::query(
                domain,
                target,
                &crate::infra::command::SystemRunner,
            )
            .await?
        }
        #[cfg(test)]
        _ => {
            plan.manager
                .query(&crate::infra::command::SystemRunner)
                .await?
        }
    };
    ensure!(state.registered, Message::ServiceNotRegistered);
    Ok(guard)
}
