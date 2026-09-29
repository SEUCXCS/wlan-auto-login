//! On-demand authorization; terminal interaction remains in the application layer.
use crate::system::define::permission::{PermissionDescriptor, PermissionProvider};
#[allow(dead_code)]
pub struct Permissions;
impl PermissionProvider for Permissions {
    fn permissions(&self) -> Vec<PermissionDescriptor> {
        vec![
            PermissionDescriptor::new("macos.location", "Location"),
            PermissionDescriptor::new("macos.keychain", "Keychain"),
        ]
    }

    #[cfg(target_os = "macos")]
    async fn status(
        &self,
        id: &crate::system::define::permission::PermissionId,
    ) -> crate::infra::error::Result<crate::system::define::permission::PermissionStatus> {
        self.validate(id)?;
        if id.0 == "macos.location" {
            return super::host::location_permission(false)
                .await
                .map(location_status);
        }
        Ok(crate::system::define::permission::PermissionStatus::NotImplemented)
    }

    #[cfg(target_os = "macos")]
    async fn request(
        &self,
        id: &crate::system::define::permission::PermissionId,
    ) -> crate::infra::error::Result<crate::system::define::permission::PermissionRequestOutcome>
    {
        use crate::system::define::permission::PermissionRequestOutcome;
        self.validate(id)?;
        if id.0 == "macos.location" {
            return super::host::location_permission(true)
                .await
                .map(location_status)
                .map(PermissionRequestOutcome::Completed);
        }
        Ok(PermissionRequestOutcome::NotImplemented)
    }
}

#[cfg(target_os = "macos")]
fn location_status(
    status: super::location::Status,
) -> crate::system::define::permission::PermissionStatus {
    use super::location::Status as L;
    use crate::system::define::permission::PermissionStatus as P;
    match status {
        L::NotDetermined => P::NotDetermined,
        L::Granted => P::Granted,
        L::Denied => P::Denied,
        L::Restricted => P::Restricted,
        L::Disabled => P::Unavailable,
        L::Unknown => P::Unknown,
    }
}
