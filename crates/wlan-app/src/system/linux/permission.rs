//! Permission declarations only; authorization is intentionally unimplemented.
use crate::system::define::permission::{PermissionDescriptor, PermissionProvider};
#[allow(dead_code)]
pub struct Permissions;
impl PermissionProvider for Permissions {
    fn permissions(&self) -> Vec<PermissionDescriptor> {
        Vec::new()
    }
}
