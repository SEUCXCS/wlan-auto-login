//! Release the inherited console from a managed Windows daemon.
use crate::infra::{error::Result, paths::Paths};
use crate::system::shared::flow::{
    hooks::Hooks,
    run::{RunContext, RunHooks, Step},
};

pub(crate) struct NativeHooks;
impl Hooks<Step, RunContext<'_>> for NativeHooks {}
impl RunHooks for NativeHooks {
    type Guard = ();

    async fn prepare(&self, _paths: &Paths, managed: bool) -> Result<()> {
        if managed {
            crate::system::windows::runtime::service::detach_console();
        }
        Ok(())
    }
}
