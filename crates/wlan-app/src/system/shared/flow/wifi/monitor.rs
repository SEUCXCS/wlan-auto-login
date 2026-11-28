//! Create a bounded monitor using the selected platform adapter.
use super::Step;
use crate::infra::error::Result;
use crate::system::flow::{NativeWifi, hooks::step};
pub(crate) use crate::system::shared::wifi::Monitor;

pub(crate) async fn run() -> Result<Monitor> {
    // Native handles and subscriptions are initialized within a bounded worker.
    step(&NativeWifi, Step::Monitor, &(), Monitor::new()).await
}
