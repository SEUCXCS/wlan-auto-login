use super::access;
use crate::infra::{error::Result, scope::Scope};
use wlanlogin_lib::Credentials;
pub async fn load() -> Result<Option<Credentials>> {
    access::load(Scope::User, false).await
}
pub async fn save(credentials: Credentials) -> Result<()> {
    access::save(Scope::User, credentials).await
}
pub async fn delete() -> Result<()> {
    access::delete(Scope::User).await
}
pub async fn load_for_daemon() -> Result<Option<Credentials>> {
    access::load(Scope::User, true).await
}
pub fn prepare_daemon() -> Result<()> {
    access::configure_interaction(false);
    let code = unsafe { security_framework_sys::keychain::SecKeychainSetUserInteractionAllowed(0) };
    crate::infra::error::ensure!(
        code == 0,
        "Failed to disable background Keychain prompts: {code}"
    );
    Ok(())
}
