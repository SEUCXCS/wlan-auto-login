//! Native daemon cancellation and credential-event capability.
use crate::infra::{error::Result, scope::Scope, shutdown::Shutdown};
pub(crate) async fn stop(shutdown: &mut Shutdown) -> Result<()> {
    shutdown.wait().await
}
#[derive(Default)]
pub(crate) struct CredentialEvents(Option<crate::system::linux::runtime::credentials::Events>);
impl CredentialEvents {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) async fn refresh(&mut self, scope: Scope) {
        if scope == Scope::User && self.0.is_none() {
            self.0 = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                crate::system::linux::runtime::credentials::Events::new(),
            )
            .await
            .ok()
            .and_then(std::result::Result::ok);
        }
    }
    pub(crate) async fn changed(&mut self) -> Result<()> {
        if let Some(events) = &mut self.0 {
            let result = events.changed().await;
            if result.is_err() {
                self.0 = None;
            }
            return result;
        }
        std::future::pending().await
    }
}
