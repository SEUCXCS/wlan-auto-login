use super::access;
use crate::infra::{
    error::{Message, Result, ensure},
    paths::Paths,
    scope::Scope,
};
use crate::system::define::credentials::CredentialMode;
use wlanlogin_lib::Credentials;
pub struct MachineStore;
impl MachineStore {
    pub fn new(_paths: Paths, mode: Option<CredentialMode>) -> Result<Self> {
        ensure!(
            !matches!(mode, Some(CredentialMode::ProtectedFile)),
            Message::ProtectedFileIsOnlyAvailableOnLinux
        );
        Ok(Self)
    }
    pub async fn load(&self, background: bool) -> Result<Option<Credentials>> {
        access::load(Scope::System, background).await
    }
    pub async fn save(&self, credentials: Credentials) -> Result<()> {
        access::save(Scope::System, credentials).await
    }
}
pub async fn delete(_paths: &Paths) -> Result<()> {
    access::delete(Scope::System).await
}
