//! Create a bounded monitor using the selected platform adapter.
use crate::infra::error::Result;
pub(crate) use crate::system::shared::wifi::Monitor;

pub(crate) async fn run() -> Result<Monitor> {
    // Native handles and subscriptions are initialized within a bounded worker.
    Monitor::new().await
}
