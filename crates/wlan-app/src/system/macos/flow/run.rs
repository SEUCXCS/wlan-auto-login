//! Hold managed launchd and Wi-Fi host guards throughout the application run.
use crate::infra::{error::Result, paths::Paths, scope::Scope};
pub(crate) use crate::system::shared::flow::run::run;
use crate::system::{
    macos::runtime::{host, service},
    shared::flow::{
        hooks::Hooks,
        run::{RunContext, RunHooks, Step},
    },
};

pub(crate) struct NativeHooks;
impl Hooks<Step, RunContext<'_>> for NativeHooks {}
impl RunHooks for NativeHooks {
    type Guard = (Option<std::fs::File>, Option<host::Host>);

    async fn prepare(&self, paths: &Paths, managed: bool) -> Result<Self::Guard> {
        let registration = if managed {
            Some(service::guard(paths).await?)
        } else {
            None
        };
        let host = if managed && paths.scope == Scope::User {
            Some(host::Host::start(host::Role::Service).await?)
        } else {
            None
        };
        Ok((registration, host))
    }
}
