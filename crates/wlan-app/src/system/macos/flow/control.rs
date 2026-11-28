//! Synchronize managed IPC and readiness with signed launchd jobs.
use crate::infra::{
    control,
    error::{Error, Result},
};
use crate::system::{
    macos::services::{bundle, readiness},
    shared::flow::{
        control::{ControlContext, ControlHooks, Step},
        hooks::Hooks,
    },
};

pub(crate) struct NativeHooks;
impl ControlHooks for NativeHooks {
    fn separate_stop(&self, context: &ControlContext<'_>) -> bool {
        bundle::enclosing(&context.plan.executable).is_some()
    }
}
impl Hooks<Step, ControlContext<'_>> for NativeHooks {
    async fn after(
        &self,
        step: Step,
        context: &ControlContext<'_>,
        result: std::result::Result<(), &Error>,
    ) -> Result<()> {
        if result.is_ok() && self.separate_stop(context) {
            match step {
                Step::StopRegistered => control::stop_managed(context.paths).await?,
                Step::Apply => readiness::wait_ready(context.paths, context.plan).await?,
                _ => {}
            }
        }
        Ok(())
    }
}
