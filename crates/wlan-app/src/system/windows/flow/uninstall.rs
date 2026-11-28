//! Windows releases its file handles before removing runtime locks.
pub(crate) use crate::system::shared::flow::uninstall::run;
use crate::system::shared::flow::{
    hooks::Hooks,
    uninstall::{Step, UninstallContext, UninstallHooks},
};

pub(crate) struct NativeHooks;
impl Hooks<Step, UninstallContext<'_>> for NativeHooks {}
impl UninstallHooks for NativeHooks {
    fn retain_locks(&self) -> bool {
        false
    }
}
