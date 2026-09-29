//! Credential storage contract shared by controllers and native backends.
use crate::infra::error::Result;
use cxxy_wlanlogin_lib::Credentials;
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum CredentialMode {
    Native,
    ProtectedFile,
}
pub trait Store {
    fn load(&self) -> impl std::future::Future<Output = Result<Option<Credentials>>>;
    fn save(&self, credentials: Credentials) -> impl std::future::Future<Output = Result<()>>;
}
