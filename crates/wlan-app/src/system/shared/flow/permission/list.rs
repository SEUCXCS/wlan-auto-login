use crate::system::define::permission::{PermissionDescriptor, PermissionProvider};

pub(crate) fn run(provider: &impl PermissionProvider) -> Vec<PermissionDescriptor> {
    // Empty capability collections are valid and never request native access.
    provider.permissions()
}
