//! Native daemon cancellation and credential-event capability.
use crate::infra::{error::Result, scope::Scope, shutdown::Shutdown};
pub(crate) async fn stop(shutdown: &mut Shutdown) -> Result<()> {
    tokio::select! { result=shutdown.wait()=>result, _=crate::system::windows::runtime::service::stop_signal()=>Ok(()) }
}
#[derive(Default)]
pub(crate) struct CredentialEvents;
impl CredentialEvents {
    pub(crate) fn new() -> Self {
        Self
    }
    pub(crate) async fn refresh(&mut self, scope: Scope) {
        let _ = scope;
    }
    pub(crate) async fn changed(&mut self) -> Result<()> {
        std::future::pending().await
    }
}
