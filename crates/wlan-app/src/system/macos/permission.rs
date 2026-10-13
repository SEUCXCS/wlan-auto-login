//! On-demand authorization; terminal interaction remains in the application layer.
use crate::system::define::permission::{PermissionDescriptor, PermissionProvider};
#[allow(dead_code)]
pub struct Permissions;
impl PermissionProvider for Permissions {
    fn permissions(&self) -> Vec<PermissionDescriptor> {
        let mut location = PermissionDescriptor::new("macos.location", "Location");
        location.description =
            Some("允许图形会话中的应用读取 Wi-Fi 名称；需要完整签名应用包。".into());
        let mut keychain = PermissionDescriptor::new("macos.keychain", "Keychain");
        keychain.description =
            Some("检查当前程序读取所选范围的已保存凭据的能力；Missing 表示尚未保存凭据。".into());
        vec![location, keychain]
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
        super::credentials::access::status(super::host::scope(), false).await
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
        super::credentials::access::status(super::host::scope(), true)
            .await
            .map(PermissionRequestOutcome::Completed)
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
