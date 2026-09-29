//! Runtime dependencies selected by the application entry point.
use crate::infra::{error::Result, paths::Paths};
use crate::system::{
    credentials::{CredentialMode, ScopedStore},
    wifi::WifiProbe,
};
use cxxy_wlanlogin_lib::{Client, PortalConfig};
pub struct Context {
    pub client: Client,
    pub paths: Paths,
}
impl Context {
    pub fn new(paths: Paths) -> Result<Self> {
        Ok(Self {
            client: Client::new(PortalConfig::default())?,
            paths,
        })
    }
    pub fn credentials(&self, mode: Option<CredentialMode>) -> Result<ScopedStore> {
        ScopedStore::new(&self.paths, mode)
    }
    pub async fn wifi(&self) -> WifiProbe {
        WifiProbe::new().await
    }
}
