//! Native Wi-Fi APIs and the signed-host monitor share one platform facade.
mod monitor;
pub(crate) mod native;
pub(crate) use monitor::Monitor;
