use super::Step;
use crate::infra::error::Result;
use crate::system::{
    define::permission::{PermissionDescriptor, PermissionProvider},
    flow::{NativePermissions, hooks::step},
};

pub(crate) async fn run(provider: &impl PermissionProvider) -> Result<Vec<PermissionDescriptor>> {
    step(&NativePermissions, Step::List, provider, async {
        Ok(provider.permissions())
    })
    .await
}
